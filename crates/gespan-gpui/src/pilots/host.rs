//! Capability-checked app actions used by Pilot provider subprocesses.
use super::*;
use gespan_core::flow::{Flow, FlowStore, RunTag};
use gespan_core::pilot::{Document, Launch, bridge::Call, completions, files, memory};
use gespan_core::{session_groups::Groups, time::now_ms};
use serde_json::{Value, json};
use std::path::Path;
/// One session in a completion report; `entry` is `None` while it is built off-thread.
pub(super) struct PendingReport {
    session: i64,
    turn: u64,
    entry: Option<String>,
}
enum Effect {
    Card(Value),
    None,
    Start(Session, Box<Settings>, String, bool, bool),
    Groups(String, Groups),
}
/// The sidebar folder holding a Pilot's sessions; the id survives renames.
pub(super) fn pilot_folder_id(pilot_id: &str) -> String {
    format!("pilot-{pilot_id}")
}
fn text(v: &Value, key: &str, max: usize) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= max)
        .map(str::to_owned)
        .ok_or_else(|| format!("Provide {key} (up to {max} bytes)"))
}
fn id(v: &Value, key: &str) -> Result<i64, String> {
    v.get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("Provide {key}"))
}
fn flag(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}
/// The Shell's Flow store handle.
type StoreHandle = std::sync::Arc<std::sync::Mutex<FlowStore>>;
const FLOWS_NOT_READY: &str = "Flows are not ready";
const FLOW_OUTSIDE_PROJECTS: &str = "Flow project is outside the assigned projects";
/// The longest goal or task a Pilot run is tagged with, in characters.
const FLOW_TAG_MAX: usize = 300;
/// The most runs `flows.runs` lists, newest first.
const FLOW_RUNS_LISTED: usize = 50;
/// Like `text`, counting characters rather than bytes.
fn short_text(v: &Value, key: &str, max: usize) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.chars().count() <= max)
        .map(str::to_owned)
        .ok_or_else(|| format!("Provide {key} (up to {max} characters)"))
}
fn flows_lock(store: &StoreHandle) -> Result<std::sync::MutexGuard<'_, FlowStore>, String> {
    store.lock().map_err(|_| "Flows are unavailable".into())
}
/// A resumed run launches sessions again, so its project must still be
/// assigned, as for `flows.start`.
fn resume_in_scope(
    action: &str,
    input: &Value,
    pilot: &pilot::Pilot,
    store: &StoreHandle,
) -> Result<(), String> {
    if action != "flows.resume" {
        return Ok(());
    }
    let run_id = text(input, "runId", 256)?;
    match flows_lock(store)?.owned_run(&pilot.id, &run_id)? {
        Some((run, _)) if !pilot.works_on(&run.flow.cwd) => Err(FLOW_OUTSIDE_PROJECTS.into()),
        _ => Ok(()),
    }
}
/// The `flow` a Pilot sent, owned by `owner` whatever it claimed. The store
/// sets the timestamps, so the Pilot may leave them out.
fn pilot_flow(v: &Value, owner: &str) -> Result<Flow, String> {
    let mut flow = v.get("flow").cloned().ok_or("Provide flow")?;
    let fields = flow.as_object_mut().ok_or("flow must be a Flow object")?;
    fields.insert("owner".into(), json!(owner));
    fields.entry("id").or_insert(json!(""));
    for key in ["createdAt", "updatedAt"] {
        fields.entry(key).or_insert(json!(0));
    }
    serde_json::from_value(flow).map_err(|e| format!("flow is not a valid Flow: {e}"))
}
/// A Flow `flows.save` may store: in one of the Pilot's projects, with an id.
fn saved_flow(v: &Value, pilot: &pilot::Pilot) -> Result<Flow, String> {
    let mut flow = pilot_flow(v, &pilot.id)?;
    if !pilot.works_on(&flow.cwd) {
        return Err(FLOW_OUTSIDE_PROJECTS.into());
    }
    if flow.id.trim().is_empty() {
        flow.id = gespan_core::automations::new_id();
    }
    Ok(flow)
}
/// The read-only `flows.*` actions and `flows.save`, against `owner`'s Flows.
fn read_flows(
    store: &FlowStore,
    owner: &str,
    action: &str,
    input: &Value,
    flow: Option<Flow>,
) -> Result<Value, String> {
    Ok(match action {
        "flows.list" => json!({"flows": store.owned(owner)?.iter().map(|f| json!({
            "id": f.id,
            "name": f.name,
            "description": f.description,
            "states": f.states.iter().map(|s| s.id()).collect::<Vec<_>>(),
            "updatedAt": f.updated_at,
        })).collect::<Vec<_>>()}),
        "flows.read" => json!(
            store
                .owned_flow(owner, &text(input, "flowId", 256)?)?
                .ok_or("Flow not found")?
        ),
        "flows.save" => {
            let flow = flow.ok_or("Provide flow")?;
            // Another Pilot's or the user's Flow is not this Pilot's to see.
            if store
                .list()?
                .iter()
                .any(|f| f.id == flow.id && f.owner.as_deref() != Some(owner))
            {
                return Err("Flow not found".into());
            }
            json!({"flow": store.upsert(&flow)?})
        }
        "flows.runs" => {
            let goal = input.get("goal").and_then(Value::as_str);
            let runs = store.owned_runs(owner, goal, FLOW_RUNS_LISTED)?;
            json!({"runs": runs.iter().map(|r| json!({
                "runId": r.id,
                "flowId": r.flow_id,
                "goal": r.tag.as_ref().map(|t| &t.goal),
                "task": r.tag.as_ref().map(|t| &t.task),
                "status": r.status,
                "currentState": r.current_state,
                "error": r.error,
            })).collect::<Vec<_>>()})
        }
        _ => {
            let (run, visits) = store
                .owned_run(owner, &text(input, "runId", 256)?)?
                .ok_or("Run not found")?;
            json!({"run": run, "visits": visits.iter().map(|v| json!({
                "seq": v.seq,
                "stateId": v.state_id,
                "status": v.status,
                "sessionId": v.session_id,
                "outputs": v.outputs,
                "error": v.error,
            })).collect::<Vec<_>>()})
        }
    })
}
fn fingerprint(call: &Call) -> String {
    files::content_hash(&format!("{}:{}", call.action, call.input))
}
impl Pilots {
    fn finish_call(&mut self, call: Call, result: Result<Value, String>, cx: &mut Context<Self>) {
        let key = (call.session, call.request_id.clone());
        self.app_pending.remove(&key);
        if self.app_results.len() > 1024
            && let Some(key) = self.app_results.keys().next().cloned()
        {
            self.app_results.remove(&key);
        }
        self.app_results
            .insert(key, (fingerprint(&call), result.clone()));
        let store = self.store.clone();
        let receipt_key = format!(
            "pilot-app-receipt:{}:{}",
            call.session,
            files::content_hash(&call.request_id)
        );
        let receipt =
            serde_json::to_string(&(fingerprint(&call), result.clone())).unwrap_or_default();
        let previous = self.writes.take();
        self.writes = Some(cx.spawn(async move |_, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx
                .background_executor()
                .spawn(async move { locked(&store)?.save_setting(&receipt_key, &receipt) });
            let reply = work
                .await
                .map(|_| ())
                .map_err(|e| format!("Could not persist app result: {e}"));
            let _ = call.reply.send(reply.and(result));
        }));
    }
    fn call_work(
        &mut self,
        call: Call,
        pilot_id: String,
        work: impl FnOnce(Database) -> Result<(Value, Effect), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move { work(store) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let result = result.map(|(value, effect)| {
                    match effect {
                        Effect::None => (),
                        Effect::Card(value) => {
                            this.hold_card(call.session, &pilot_id, value);
                            this.save(cx);
                        }
                        Effect::Groups(project, groups) => {
                            let _ = this.owner.update(cx, |owner, cx| {
                                owner.pilot_apply_groups(project, groups, cx)
                            });
                        }
                        Effect::Start(summary, settings, prompt, draft, reveal) => {
                            let id = summary.id;
                            let caller = call.session;
                            let origin = this
                                .chats
                                .values()
                                .chain(this.running.values().map(|r| &r.view))
                                .find(|v| v.read(cx).summary.id == caller)
                                .map(|v| v.read(cx).pilot_turn)
                                .unwrap_or(0);
                            if let Some(pilot) = this.roster.get_mut(&pilot_id) {
                                pilot.launches.push(Launch {
                                    session_id: id,
                                    cwd: summary.cwd.clone(),
                                    title: summary.title.clone(),
                                    harness: summary.provider.clone(),
                                    model: settings.model.clone(),
                                    origin_turn: ((caller as u64) << 32) | origin,
                                    notify_on_complete: flag(
                                        &call.input,
                                        "notifyOnComplete",
                                        !draft,
                                    ),
                                    completed: false,
                                    dismissed: false,
                                });
                            }
                            let _ = this.owner.update(cx, |owner, cx| {
                                owner.pilot_install(summary, *settings, prompt, draft, cx)
                            });
                            this.save(cx);
                            this.group_launch(&pilot_id, id, cx);
                            if reveal {
                                this.open_workspace(id, window, cx);
                            }
                        }
                    }
                    this.reload_files(&pilot_id, cx);
                    this.changed(cx);
                    value
                });
                this.finish_call(call, result, cx);
            });
        })
        .detach();
    }
    pub(super) fn app_call(&mut self, call: Call, window: &mut Window, cx: &mut Context<Self>) {
        let pilot = self
            .roster
            .of_session(call.session)
            .or_else(|| {
                self.running
                    .get(&call.session)
                    .and_then(|r| self.roster.get(&r.pilot))
            })
            .or_else(|| {
                self.reviews
                    .get(&call.session)
                    .and_then(|r| self.roster.get(&r.pilot))
            })
            .cloned();
        let Some(pilot) = pilot else {
            let _ = call.reply.send(Err(
                "This Pilot no longer owns an active app capability".into()
            ));
            return;
        };
        if call.action.starts_with("habits.")
            && call.action != "habits.list"
            && self.running.contains_key(&call.session)
        {
            let _ = call
                .reply
                .send(Err("A habit run cannot schedule more runs".into()));
            return;
        }
        if self.reviews.contains_key(&call.session)
            && !flow_reviews::REVIEW_ACTIONS.contains(&call.action.as_str())
        {
            let _ = call.reply.send(Err(flow_reviews::REVIEW_READ_ONLY.into()));
            return;
        }
        if let Err(error) = pilot::api::validate(&call.action, &call.input) {
            let _ = call.reply.send(Err(error));
            return;
        }
        let receipt_key = format!(
            "pilot-app-receipt:{}:{}",
            call.session,
            files::content_hash(&call.request_id)
        );
        let stored = locked(&self.store)
            .ok()
            .and_then(|db| db.setting(&receipt_key).ok().flatten())
            .and_then(|s| serde_json::from_str::<(String, Result<Value, String>)>(&s).ok());
        if let Some((hash, result)) = stored {
            let _ = call.reply.send(if hash == fingerprint(&call) {
                result
            } else {
                Err("Request ID was already used with different input".into())
            });
            return;
        }
        let key = (call.session, call.request_id.clone());
        if let Some((hash, result)) = self.app_results.get(&key) {
            let _ = call.reply.send(if hash == &fingerprint(&call) {
                result.clone()
            } else {
                Err("Request ID was already used with different input".into())
            });
            return;
        }
        if !self.app_pending.insert(key) {
            let _ = call.reply.send(Err(
                "Request is still running; retry with the same request ID".into(),
            ));
            return;
        }
        if !call.input.is_object() {
            self.finish_call(call, Err("Input must be a JSON object".into()), cx);
            return;
        }
        let input = call.input.clone();
        let action = call.action.clone();
        let root = self.root.clone();
        let target = pilot.id.clone();
        let project = if action.starts_with("sessions.")
            || action.starts_with("folders.")
            || action.starts_with("worktrees.")
        {
            match pilot.project(input.get("project").and_then(Value::as_str)) {
                Ok(p) => p,
                Err(e) => {
                    self.finish_call(call, Err(e), cx);
                    return;
                }
            }
        } else {
            String::new()
        };
        let hidden = self.roster.hidden_sessions.clone();
        let source = self
            .chats
            .get(&pilot.id)
            .or_else(|| self.running.get(&call.session).map(|r| &r.view))
            .cloned();
        let result=match action.as_str(){
            "models.list"=>Ok(json!(Kind::ALL.into_iter().map(|kind|json!({"harness":kind.id(),"name":kind.label(),"models":self.chats.values().find(|v|{let v=v.read(cx);let profile=source.as_ref().filter(|s|s.read(cx).summary.provider==kind.id()).map(|s|s.read(cx).settings.account.clone()).unwrap_or_else(||crate::accounts::get(cx).map(|center|center.read(cx).selected(kind,&project)).unwrap_or_default());v.summary.provider==kind.id()&&v.settings.account.id==profile.id}).map(|v|v.read(cx).settings_models()).unwrap_or_default()})).collect::<Vec<_>>())),
            "sessions.start"=>{
                let prompt=match text(&input,"prompt",65536){Ok(t)=>t,Err(e)=>{self.finish_call(call,Err(e),cx);return;}};
                let provider=input.get("harness").and_then(Value::as_str).map(str::to_owned).or_else(||source.as_ref().map(|v|v.read(cx).summary.provider.clone())).unwrap_or_else(||"codex".into());
                if Kind::parse(&provider).is_none(){self.finish_call(call,Err("Unknown provider".into()),cx);return;}
                let same_provider=source.as_ref().is_some_and(|v| v.read(cx).summary.provider==provider);
                let mut settings=source.map(|v|v.read(cx).settings.clone()).unwrap_or_default();settings.pilot=None;settings.environment.clear();settings.internal_message=false;settings.turns.clear();settings.attachments.clear();settings.usage_limit=None;
                if !same_provider { settings.account=crate::accounts::get(cx).map(|center|center.read(cx).selected(Kind::from_id(&provider),&project)).unwrap_or_default();settings.model=None;settings.effort=None;settings.model_settings.clear();settings.fast=false;settings.service_tier=None;settings.thinking=None; }
                if let Some(model)=input.get("model").and_then(Value::as_str){settings.model=Some(model.into());}
                if let Some(values)=input.get("modelSettings"){match serde_json::from_value(values.clone()){Ok(values)=>settings.model_settings=values,Err(_)=>{self.finish_call(call,Err("modelSettings must be an object of strings".into()),cx);return;}}}
                if let Some(mode)=input.get("runtimeMode").and_then(Value::as_str){settings.access=AccessMode::from_id(mode);}
                let draft=flag(&input,"draft",false);let reveal=flag(&input,"reveal",false);
                self.call_work(call,target,move|store|{
                    let db=locked(&store)?;let mut summary=db.create(&provider,&project)?;
                    if let Some(cwd)=input.get("worktreeCwd").and_then(Value::as_str){let snapshot=gespan_core::git::snapshot(Path::new(&project))?;if !snapshot.worktrees.iter().any(|w|gespan_core::files::path_key(&w.path)==gespan_core::files::path_key(cwd)){return Err("Choose an existing worktree of the assigned project".into());}settings.worktree_cwd=Some(cwd.into());}
                    else if input.get("workspaceMode").and_then(Value::as_str)==Some("worktree"){settings.worktree_cwd=Some(gespan_core::git::create_worktree(Path::new(&project),input.get("worktreeBase").and_then(Value::as_str).unwrap_or("HEAD"),summary.id)?.to_string_lossy().into());}
                    summary.title=input.get("title").and_then(Value::as_str).unwrap_or(&prompt).chars().take(160).collect();db.title(summary.id,&summary.title)?;db.configure_agent(summary.id,&provider,&settings)?;
                    Ok((json!({"sessionId":summary.id,"accepted":true,"project":project}),Effect::Start(summary,Box::new(settings),prompt,draft,reveal)))
                },cx);return;
            }
            "sessions.list"|"sessions.read"=>{
                self.call_work(call,target,move|store|{let db=locked(&store)?;let all=db.sessions()?;let sessions:Vec<_>=all.iter().filter(|s|!hidden.contains(&s.id)&&gespan_core::files::path_key(&s.cwd)==gespan_core::files::path_key(&project)).collect();
                    let value=if action=="sessions.list"{json!(sessions.iter().map(|s|json!({"sessionId":s.id,"title":s.title,"harness":s.provider,"project":s.cwd,"archived":s.archived})).collect::<Vec<_>>())}else{let session=id(&input,"sessionId")?;if !sessions.iter().any(|s|s.id==session){return Err("Session is outside the assigned project".into());}let rows=db.load(session)?.rows;let end=input.get("before").and_then(Value::as_u64).unwrap_or(rows.len() as u64).min(rows.len() as u64) as usize;let limit=input.get("limit").and_then(Value::as_u64).unwrap_or(40).clamp(1,200) as usize;let start=end.saturating_sub(limit);let max=input.get("maxChars").and_then(Value::as_u64).unwrap_or(24000).clamp(100,200000) as usize;json!({"sessionId":session,"messages":rows[start..end].iter().map(|r|json!({"role":r.role,"text":gespan_core::response::limit(&r.text,max/limit.max(1))})).collect::<Vec<_>>(),"before":start,"hasMore":start>0})};Ok((value,Effect::None))},cx);return;
            }
            "sessions.send"|"sessions.draft"|"sessions.stop"|"sessions.archive"|"sessions.delete"=>{
                let session=match id(&input,"sessionId"){Ok(id)=>id,Err(e)=>{self.finish_call(call,Err(e),cx);return;}};
                let view=self.owner.upgrade().and_then(|owner|owner.update(cx, |owner, cx| owner.pilot_ensure_session(session, cx))).filter(|v|gespan_core::files::path_key(&v.read(cx).summary.cwd)==gespan_core::files::path_key(&project));
                let Some(view)=view else{self.finish_call(call,Err("Session is outside the assigned project".into()),cx);return;};
                if action=="sessions.send"||action=="sessions.draft"{if view.read(cx).busy||view.read(cx).pilot_external_pending()||!view.read(cx).input.read(cx).content.trim().is_empty(){self.finish_call(call,Err("Session is busy or has an unsent draft".into()),cx);return;}match text(&input,"prompt",65536){Ok(prompt)=>{let sent=view.update(cx,|v,cx|v.pilot_external_send(prompt,action=="sessions.draft",cx));if let Err(e)=sent{self.finish_call(call,Err(e),cx);return;}
if flag(&input,"notifyOnComplete",false){if let Some(m)=self.roster.get_mut(&pilot.id){let origin=source.map(|v|v.read(cx).pilot_turn).unwrap_or(0);if let Some(launch)=m.launches.iter_mut().find(|l|l.session_id==session){launch.completed=false;launch.dismissed=false;launch.notify_on_complete=true;launch.origin_turn=((call.session as u64)<<32)|origin;}else{let summary=&view.read(cx).summary;m.launches.push(Launch{session_id:session,cwd:summary.cwd.clone(),title:summary.title.clone(),harness:summary.provider.clone(),model:view.read(cx).pilot_model(),origin_turn:((call.session as u64)<<32)|origin,notify_on_complete:true,completed:false,dismissed:false});}}self.save(cx);let loose=self.owner.upgrade().is_some_and(|o|o.read(cx).pilot_groups(&project).folder_of(session).is_none());if loose{let pid=pilot.id.clone();self.group_launch(&pid,session,cx);}}Ok(json!({"accepted":true}))},Err(e)=>Err(e)}}else{
                    for m in &mut self.roster.pilots{for launch in &mut m.launches{if launch.session_id==session{launch.dismissed=true;}}}self.dismiss_reports(session,cx);self.save(cx);
                    if action=="sessions.stop"{view.update(cx,|v,cx|v.stop_if_busy(cx));}else{let delete=action=="sessions.delete";let owner=self.owner.clone();let handle=window.window_handle();let app:&mut App=cx;app.defer(move|cx|{let _=handle.update(cx,|_,window,cx|{let _=owner.update(cx,|owner,cx|owner.pilot_remove(session,delete,window,cx));});});}Ok(json!({"accepted":true}))
                }
            }
            "folders.list"=>Ok(json!(self.owner.upgrade().map(|o|o.read(cx).pilot_groups(&project)).unwrap_or_default().folders)),
            "folders.move"=>{
                let session=match id(&input,"sessionId"){Ok(id)=>id,Err(e)=>{self.finish_call(call,Err(e),cx);return;}};
                let allowed=self.owner.upgrade().and_then(|o|o.update(cx, |owner, cx| owner.pilot_ensure_session(session, cx))).is_some_and(|v|v.read(cx).summary.cwd==project);
                if !allowed{self.finish_call(call,Err("Session is outside this project".into()),cx);return;}
                let mut groups=self.owner.upgrade().map(|o|o.read(cx).pilot_groups(&project)).unwrap_or_default();
                if let Some(name)=input.get("newFolderName").and_then(Value::as_str){if let Some(folder)=groups.create_folder(&[session],pilot::new_id()){groups.rename_folder(&folder,name);}}
                else if let Some(folder)=input.get("folderId").and_then(Value::as_str){if groups.folder(folder).is_none(){self.finish_call(call,Err("Folder not found".into()),cx);return;}groups.add_to_folder(folder,session);}else{groups.remove_sessions(&[session]);}
                self.call_work(call,target,move|store|{locked(&store)?.save_groups(&project,&groups)?;Ok((json!({"moved":true}),Effect::Groups(project,groups)))},cx);return;
            }
            "artifacts.list"=>Ok(json!(pilot.documents.iter().filter(|d|input.get("kind").and_then(Value::as_str).is_none_or(|k|k==d.kind)).skip(input.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize).take(input.get("limit").and_then(Value::as_u64).unwrap_or(50).min(200) as usize).collect::<Vec<_>>())),
            "artifacts.read"=>input.get("id").and_then(Value::as_str).and_then(|id|pilot.documents.iter().find(|d|d.id==id)).map(|d|json!(d)).ok_or("Document not found".into()),
            "chat.card"=>{
                let mut card=match pilot::cards::parse(&input){Ok(c)=>c,Err(e)=>{self.finish_call(call,Err(e),cx);return;}};
                card["id"]=json!(pilot::new_id());
                if card["type"]=="session"{
                    let session=card["sessionId"].as_i64().unwrap();
                    let view=self.owner.upgrade().and_then(|o|o.update(cx, |owner, cx| owner.pilot_ensure_session(session, cx))).filter(|v|pilot.works_on(&v.read(cx).summary.cwd));
                    let Some(view)=view else{self.finish_call(call,Err("Session is outside assigned projects".into()),cx);return;};
                    card["title"]=json!(view.read(cx).summary.title);card["state"]=json!(if view.read(cx).busy{"Working"}else{"Saved"});
                }
                if card["type"]=="pr"{
                    let cwd=pilot.projects.first().cloned().unwrap_or_else(||std::env::var("HOME").unwrap_or_else(|_|".".into()));
                    self.call_work(call,target,move|_|{let metadata=pilot::cards::pull_request(&cwd,card["repo"].as_str(),card["number"].as_u64().unwrap())?;for key in ["title","state","url","isDraft","additions","deletions"]{card[key]=metadata[key].clone();}Ok((card.clone(),Effect::Card(card)))},cx);return;
                }
                self.hold_card(call.session,&pilot.id,card.clone());self.save(cx);Ok(card)
            }
            "artifacts.write"=>{
                let kind=input.get("kind").and_then(Value::as_str).unwrap_or("document");
                if !["document","html","markdown"].contains(&kind){Err("Use document or html".into())}else{match text(&input,"title",512).and_then(|title|Ok((title,text(&input,"body",512*1024)?))){Err(e)=>Err(e),Ok((title,body))=>{
                    let id=input.get("id").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(pilot::new_id);let now=now_ms();let document=Document{id:id.clone(),kind:kind.into(),title,summary:input.get("summary").and_then(Value::as_str).unwrap_or_default().chars().take(4000).collect(),body,created_at:pilot.documents.iter().find(|d|d.id==id).map(|d|d.created_at).unwrap_or(now),updated_at:now};
                    if let Some(m)=self.roster.get_mut(&pilot.id){if let Some(old)=m.documents.iter_mut().find(|d|d.id==id){*old=document.clone();}else{m.documents.push(document.clone());}}
                    self.documents.remove(&id);self.html_documents.remove(&id);self.hold_card(call.session,&pilot.id,json!({"id":pilot::new_id(),"type":"document","documentId":id,"title":document.title,"summary":document.summary}));self.save(cx);Ok(json!(document))
                }}}
            }
            "soul.read"|"soul.update"|"memory.read"|"memory.search"|"memory.add"|"memory.replace"|"memory.remove"|"habits.list"|"habits.add"|"habits.update"|"habits.run"|"habits.remove"|"notes.list"|"notes.read"|"notes.write"|"worktrees.list"|"worktrees.create"=>{
                let caller=call.session;let projects=pilot.projects.clone();
                self.call_work(call,target.clone(),move|store|{
                    let value=match action.as_str(){
                        "soul.read"=>json!(files::read(&root,&target,None,"SOUL.md")?),
                        "soul.update"=>{let body=input.get("text").and_then(Value::as_str).filter(|t|t.len()<=128*1024).ok_or("Provide text")?;let expected=text(&input,"expectedHash",128)?;json!({"hash":files::save(&root,&target,None,"SOUL.md",body,Some(&expected))?})},
                        "memory.read"=>json!(memory::read(&root,&target,input.get("topic").and_then(Value::as_str))?),
                        "memory.search"=>json!(memory::search(&root,&target,&text(&input,"query",4000)?,input.get("since").and_then(Value::as_str))?),
                        "memory.add"|"memory.replace"|"memory.remove"=>{let entry=if action=="memory.remove"{None}else{Some(memory::entry(&text(&input,"fact",4000)?,&memory::date(),input.get("until").and_then(Value::as_str))?)};let find=input.get("find").and_then(Value::as_str).unwrap_or_default();let hash=memory::edit(&root,&target,input.get("topic").and_then(Value::as_str),|old|if action=="memory.add"{Ok(memory::add(old,entry.as_deref().unwrap()))}else{memory::replace(old,find,entry.as_deref(),&memory::date())})?;json!({"hash":hash})},
                        "habits.list"=>json!(habits::load(&root,&target)?),
                        "habits.add"=>{let schedule:habits::Schedule=serde_json::from_value(input.get("schedule").cloned().ok_or("Provide schedule")?).map_err(|e|e.to_string())?;let h=Habit::new(text(&input,"name",480)?,text(&input,"instructions",32000)?,schedule,now_ms())?;habits::update(&root,&target,|all|{all.push(h.clone());Ok(())})?;json!(h)},
                        "habits.update"|"habits.run"|"habits.remove"=>{let id=text(&input,"id",128)?;habits::update(&root,&target,|all|{let index=all.iter().position(|h|h.id==id).ok_or("Habit not found")?;if action=="habits.remove"{all.remove(index);return Ok(json!({"removed":true}));}let h=&mut all[index];if action=="habits.run"{h.run_requested=true;}else{if let Some(name)=input.get("name").and_then(Value::as_str){if name.trim().is_empty()||name.chars().count()>120{return Err("Invalid habit name".into());}h.name=name.trim().into();}
if let Some(instructions)=input.get("instructions").and_then(Value::as_str){if instructions.trim().is_empty()||instructions.len()>32000{return Err("Invalid habit instructions".into());}h.instructions=instructions.into();}
if let Some(schedule)=input.get("schedule"){h.schedule=serde_json::from_value(schedule.clone()).map_err(|e|e.to_string())?;h.schedule.validate()?;h.next_run_at=h.schedule.next(now_ms())?;}
if let Some(enabled)=input.get("enabled").and_then(Value::as_bool){h.enabled=enabled;if enabled{h.next_run_at=h.schedule.next(now_ms())?;}}}Ok(json!(h))})?},
                        "notes.list"|"notes.read"|"notes.write"=>{let db=locked(&store)?;let notes=db.notes()?;let allowed=|n:&&gespan_core::response::Note|projects.iter().any(|p|gespan_core::files::path_key(p)==gespan_core::files::path_key(&n.source_cwd));match action.as_str(){"notes.list"=>json!(notes.iter().filter(allowed).skip(input.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize).take(input.get("limit").and_then(Value::as_u64).unwrap_or(50).min(200) as usize).collect::<Vec<_>>()),"notes.read"=>{let id=id(&input,"id")? as u64;json!(notes.iter().filter(allowed).find(|n|n.id==id).ok_or("Note not found in assigned projects")?)},_=>{let id=input.get("id").and_then(Value::as_u64).unwrap_or_else(now_ms);if notes.iter().any(|n|n.id==id&&!projects.iter().any(|p|p==&n.source_cwd)){return Err("Note is outside assigned projects".into());}let cwd=notes.iter().find(|n|n.id==id).map(|n|n.source_cwd.clone()).or_else(||projects.first().cloned()).ok_or("Assign a project before writing notes")?;let note=gespan_core::response::Note{id,title:text(&input,"title",512)?,body:text(&input,"body",256*1024)?,tags:input.get("tags").and_then(Value::as_array).map(|tags|tags.iter().filter_map(Value::as_str).map(str::to_owned).collect()).or_else(||notes.iter().find(|n|n.id==id).map(|n|n.tags.clone())).unwrap_or_default(),source_session:caller,source_cwd:cwd};db.save_note(&note)?;json!(note)}}},
                        "worktrees.list"=>json!(gespan_core::git::snapshot(Path::new(&project))?.worktrees.iter().map(|w|json!({"path":w.path,"branch":w.branch,"head":w.head,"main":w.main})).collect::<Vec<_>>()),
                        "worktrees.create"=>{let branch=text(&input,"branch",240)?;let tree=gespan_core::git::create_project_worktree(Path::new(&project),&branch,input.get("base").and_then(Value::as_str),flag(&input,"existing",false))?;json!({"path":tree.path,"branch":tree.branch})},
                        _=>unreachable!(),
                    };Ok((value,Effect::None))
                },cx);return;
            }
            a if a.starts_with("flows.")=>{self.flows_call(call,&pilot,cx);return;}
            _=>Err("Unknown action. Run app help to see supported actions".into()),
        };
        self.finish_call(call, result, cx);
    }
    /// `flows.*`: the calling Pilot's own Flows and runs, never anyone else's.
    /// Reads and saves run off-thread; run changes go through the Shell.
    fn flows_call(&mut self, call: Call, pilot: &pilot::Pilot, cx: &mut Context<Self>) {
        let store = self
            .owner
            .upgrade()
            .ok_or_else(|| FLOWS_NOT_READY.to_string())
            .and_then(|shell| shell.read(cx).pilot_flows_store());
        let store = match store {
            Ok(store) => store,
            Err(error) => return self.finish_call(call, Err(error), cx),
        };
        let input = call.input.clone();
        let action = call.action.clone();
        let owner = pilot.id.clone();
        let result = match action.as_str() {
            "flows.list" | "flows.read" | "flows.runs" | "flows.run" | "flows.save" => {
                let flow = match action.as_str() {
                    "flows.save" => match saved_flow(&input, pilot) {
                        Ok(flow) => Some(flow),
                        Err(error) => return self.finish_call(call, Err(error), cx),
                    },
                    _ => None,
                };
                let target = owner.clone();
                self.call_work(
                    call,
                    target,
                    move |_| {
                        let store = flows_lock(&store)?;
                        Ok((
                            read_flows(&store, &owner, &action, &input, flow)?,
                            Effect::None,
                        ))
                    },
                    cx,
                );
                return;
            }
            "flows.start" => self.start_flow_run(&input, pilot, &store, cx),
            "flows.editRun" | "flows.cancel" | "flows.resume" | "flows.delete" => {
                let changed = resume_in_scope(&action, &input, pilot, &store)
                    .and_then(|()| self.change_flows(&action, &input, &owner, cx));
                if changed.is_ok()
                    && let Some(run_id) = input.get("runId").and_then(Value::as_str)
                {
                    self.flow_changed(&owner, &action, run_id);
                    self.save(cx);
                }
                changed
            }
            _ => Err("Unknown action. Run app help to see supported actions".into()),
        };
        self.finish_call(call, result, cx);
    }
    fn start_flow_run(
        &mut self,
        input: &Value,
        pilot: &pilot::Pilot,
        store: &StoreHandle,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let flow_id = text(input, "flowId", 256)?;
        let tag = RunTag {
            goal: short_text(input, "goal", FLOW_TAG_MAX)?,
            task: short_text(input, "task", FLOW_TAG_MAX)?,
        };
        let inputs: HashMap<String, String> = match input.get("inputs") {
            Some(inputs) => serde_json::from_value(inputs.clone())
                .map_err(|_| "inputs must be an object of text values".to_string())?,
            None => HashMap::new(),
        };
        // Its project may have been unassigned since the Flow was saved.
        let cwd = flows_lock(store)?
            .owned_flow(&pilot.id, &flow_id)?
            .ok_or("Flow not found")?
            .cwd;
        if !pilot.works_on(&cwd) {
            return Err(FLOW_OUTSIDE_PROJECTS.into());
        }
        let owner = pilot.id.clone();
        let run = self
            .owner
            .update(cx, |shell, cx| {
                shell.start_pilot_flow(&owner, &flow_id, inputs, tag, cx)
            })
            .map_err(|_| FLOWS_NOT_READY.to_string())?
            .map_err(|errors| errors.join("; "))?;
        let id = run.id.clone();
        self.flow_started(&owner, run);
        self.save(cx);
        Ok(json!({"runId": id}))
    }
    fn change_flows(
        &mut self,
        action: &str,
        input: &Value,
        owner: &str,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let (result, done) = match action {
            "flows.delete" => {
                let flow_id = text(input, "flowId", 256)?;
                let result = self
                    .owner
                    .update(cx, |shell, _| shell.delete_pilot_flow(owner, &flow_id));
                (result, json!({"deleted": true}))
            }
            "flows.editRun" => {
                let run_id = text(input, "runId", 256)?;
                let flow = pilot_flow(input, owner)?;
                let note = text(input, "note", 4000)?;
                let result = self.owner.update(cx, |shell, cx| {
                    shell
                        .edit_pilot_run(owner, &run_id, flow, &note, cx)
                        .map_err(|errors| errors.join("; "))
                });
                (result, json!({"edited": true}))
            }
            "flows.cancel" => {
                let run_id = text(input, "runId", 256)?;
                let result = self
                    .owner
                    .update(cx, |shell, cx| shell.cancel_pilot_run(owner, &run_id, cx));
                (result, json!({"cancelled": true}))
            }
            _ => {
                let run_id = text(input, "runId", 256)?;
                let state_id = text(input, "stateId", 256)?;
                let result = self.owner.update(cx, |shell, cx| {
                    shell.resume_pilot_run(owner, &run_id, &state_id, cx)
                });
                (result, json!({"resumed": true}))
            }
        };
        result.map_err(|_| FLOWS_NOT_READY.to_string())??;
        Ok(done)
    }
    /// Files a Pilot-started session in its folder; every launch is filed.
    pub(super) fn group_launch(&mut self, id: &str, session: i64, cx: &mut Context<Self>) {
        let Some(launch) = self
            .roster
            .get(id)
            .and_then(|p| p.launches.iter().find(|l| l.session_id == session))
        else {
            return;
        };
        let project = launch.cwd.clone();
        self.file_session(id, project, session, cx);
    }
    /// Files a session a Pilot's Flow run started in the Pilot's folder. It is not a launch
    /// (the Flow's events report it), so it never joins `launches`.
    pub(crate) fn file_flow_session(
        &mut self,
        id: &str,
        project: String,
        session: i64,
        cx: &mut Context<Self>,
    ) {
        self.file_session(id, project, session, cx);
    }
    /// Puts `session` in Pilot `id`'s folder of `project`.
    fn file_session(&mut self, id: &str, project: String, session: i64, cx: &mut Context<Self>) {
        let Some(pilot) = self.roster.get(id).cloned() else {
            return;
        };
        let mut groups = self
            .owner
            .upgrade()
            .map(|o| o.read(cx).pilot_groups(&project))
            .unwrap_or_default();
        file_in_pilot_folder(&mut groups, &pilot, session);
        self.apply_groups(project, groups, cx);
    }
    /// Files launches that versions before every launch was filed left out of folders: each
    /// one whose session still exists and is in no folder goes into its Pilot's folder.
    /// Sessions in any folder stay put, so running it again changes nothing.
    pub(super) fn file_loose_launches(&mut self, cx: &mut Context<Self>) {
        let projects: HashSet<String> = self
            .roster
            .pilots
            .iter()
            .flat_map(|p| p.launches.iter().map(|l| l.cwd.clone()))
            .collect();
        if projects.is_empty() {
            return;
        }
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let db = locked(&store)?;
            let sessions: HashSet<i64> = db.sessions()?.into_iter().map(|s| s.id).collect();
            let groups = projects
                .into_iter()
                .map(|p| db.groups(&p).map(|g| (p, g)))
                .collect::<Result<HashMap<_, _>, String>>()?;
            Ok::<_, String>((sessions, groups))
        });
        cx.spawn(async move |this, cx| {
            let Ok((sessions, saved)) = work.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.file_launches(&sessions, saved, cx));
        })
        .detach();
    }
    fn file_launches(
        &mut self,
        sessions: &HashSet<i64>,
        saved: HashMap<String, Groups>,
        cx: &mut Context<Self>,
    ) {
        let owner = self.owner.upgrade();
        // The sidebar's copy wins for projects it has open; it may have moved on since startup.
        let mut groups: HashMap<String, Groups> = saved
            .into_iter()
            .map(|(project, saved)| {
                let open = owner
                    .as_ref()
                    .and_then(|o| o.read(cx).pilot_loaded_groups(&project));
                (project, open.unwrap_or(saved))
            })
            .collect();
        let mut changed = HashSet::new();
        for pilot in &self.roster.pilots {
            for launch in &pilot.launches {
                let Some(project) = groups.get_mut(&launch.cwd) else {
                    continue;
                };
                if sessions.contains(&launch.session_id)
                    && project.folder_of(launch.session_id).is_none()
                {
                    file_in_pilot_folder(project, pilot, launch.session_id);
                    changed.insert(launch.cwd.clone());
                }
            }
        }
        for project in changed {
            let filed = groups.remove(&project).unwrap_or_default();
            self.apply_groups(project, filed, cx);
        }
    }
    fn apply_groups(&mut self, project: String, groups: Groups, cx: &mut Context<Self>) {
        let _ = self.owner.update(cx, |o, cx| {
            o.pilot_apply_groups(project.clone(), groups.clone(), cx)
        });
        let store = self.store.clone();
        cx.background_executor()
            .spawn(async move {
                let _ = locked(&store).and_then(|db| db.save_groups(&project, &groups));
            })
            .detach();
    }
    pub(super) fn check_completions(&mut self, cx: &mut Context<Self>) {
        let views = self
            .owner
            .upgrade()
            .map(|o| o.read(cx).pilot_session_views(cx))
            .unwrap_or_default();
        let source_busy = |source: i64| {
            self.chats
                .values()
                .chain(self.running.values().map(|r| &r.view))
                .any(|v| v.read(cx).summary.id == source && v.read(cx).busy)
        };
        let mut ready = Vec::new();
        for pilot in &self.roster.pilots {
            let stopped = |session: i64| {
                views
                    .iter()
                    .find(|v| v.id == session)
                    .is_none_or(|v| !v.busy && v.has_messages && !v.held)
            };
            let in_flight = |session: i64| {
                self.completion_work.iter().any(|((p, _), work)| {
                    *p == pilot.id && work.iter().any(|w| w.session == session)
                })
            };
            for report in completions::ready(&pilot.launches, stopped, source_busy, in_flight) {
                let children: Vec<_> = report
                    .launches
                    .iter()
                    .filter_map(|&(session, _)| {
                        pilot
                            .launches
                            .iter()
                            .find(|l| l.session_id == session)
                            .map(|l| (session, l.title.clone()))
                    })
                    .collect();
                ready.push((pilot.id.clone(), report, children));
            }
        }
        for (id, report, children) in ready {
            let key = (id, report.source);
            self.completion_work.entry(key.clone()).or_default().extend(
                report
                    .launches
                    .iter()
                    .map(|&(session, turn)| PendingReport {
                        session,
                        turn,
                        entry: None,
                    }),
            );
            let store = self.store.clone();
            let work = cx.background_executor().spawn(async move {
                children
                    .into_iter()
                    .map(|(id, title)| {
                        let rows = locked(&store)
                            .and_then(|db| db.load(id))
                            .map(|t| gespan_core::response::prose(&t.rows))
                            .unwrap_or_else(|_| "Session was removed".into());
                        let entry = format!(
                            "Session {id}: {title}\n{}",
                            gespan_core::response::limit(&rows, 12000)
                        );
                        (id, entry)
                    })
                    .collect::<Vec<_>>()
            });
            cx.spawn(async move |this, cx| {
                let entries = work.await;
                let _ = this.update(cx, |this, cx| this.finish_completion(key, entries, cx));
            })
            .detach();
        }
    }
    /// Holds a chat's built reports until every report for it is built, so sessions that stop
    /// close together reach it as one message.
    fn finish_completion(
        &mut self,
        key: (String, i64),
        entries: Vec<(i64, String)>,
        cx: &mut Context<Self>,
    ) {
        let Some(work) = self.completion_work.get_mut(&key) else {
            return;
        };
        for (session, entry) in entries {
            if let Some(w) = work
                .iter_mut()
                .find(|w| w.session == session && w.entry.is_none())
            {
                w.entry = Some(entry);
            }
        }
        let building = |this: &Self| this.completion_work[&key].iter().any(|w| w.entry.is_none());
        if building(self) {
            return;
        }
        // Siblings that stopped while this was built join the same message.
        self.check_completions(cx);
        if building(self) {
            return;
        }
        let work = self.completion_work.remove(&key).unwrap_or_default();
        let (id, source) = key;
        let mut entries = Vec::new();
        if let Some(pilot) = self.roster.get_mut(&id) {
            for w in work {
                // A session the Pilot stopped, archived or deleted meanwhile never reports.
                let Some(launch) = pilot
                    .launches
                    .iter_mut()
                    .find(|l| l.session_id == w.session && !l.dismissed)
                else {
                    continue;
                };
                // Only the reported turn closes; a launch re-armed since stays open.
                if launch.origin_turn == w.turn {
                    launch.completed = true;
                }
                entries.extend(w.entry.map(|entry| (w.session, entry)));
            }
        }
        if entries.is_empty() {
            self.save(cx);
            return;
        }
        if let Some(run) = self.running.get_mut(&source) {
            match run.mailbox.iter_mut().find(|m| !m.report.is_empty()) {
                Some(queued) => {
                    completions::merge_report(&mut queued.report, entries);
                    rewrite_report(queued);
                }
                None => run.mailbox.push(new_report(entries)),
            }
            self.save(cx);
            self.habit_changed(source, cx);
            return;
        }
        // A report still waiting in the outbox absorbs these sessions; one already handed to
        // the chat is never rewritten.
        let handed = self.handed_reports(&id, cx);
        if let Some(pilot) = self.roster.get_mut(&id) {
            match pilot.outbox.iter_mut().find(|m| {
                m.internal && m.error.is_none() && !m.report.is_empty() && !handed.contains(&m.id)
            }) {
                Some(queued) => {
                    completions::merge_report(&mut queued.report, entries);
                    rewrite_report(queued);
                }
                None => pilot.outbox.push(new_report(entries)),
            }
            self.save(cx);
            self.configure_chat(&id, cx);
            self.dispatch(&id, cx);
        }
        self.ensure_chat(&id, cx);
    }
    /// Reports the chat has been handed, delivered or in flight; they are never rewritten.
    fn handed_reports(&self, id: &str, cx: &App) -> HashSet<String> {
        self.chats
            .get(id)
            .map(|v| v.read(cx).pilot_delivered().clone())
            .unwrap_or_default()
            .into_iter()
            .chain(self.inflight.get(id).map(|m| m.id.clone()))
            .collect()
    }
    /// Takes a session the Pilot stopped, archived or deleted out of every report not handed
    /// over yet, so it never reports; a report left naming no session is dropped.
    fn dismiss_reports(&mut self, session: i64, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.roster.pilots.iter().map(|p| p.id.clone()).collect();
        for id in ids {
            let handed = self.handed_reports(&id, cx);
            let Some(pilot) = self.roster.get_mut(&id) else {
                continue;
            };
            if dismiss_from(&mut pilot.outbox, session, |m| !handed.contains(&m.id)) {
                self.configure_chat(&id, cx);
                self.dispatch(&id, cx);
            }
        }
        // A habit run only waits for its mailbox, so one emptied here may be done.
        let runs: Vec<i64> = self
            .running
            .iter_mut()
            .filter_map(|(&run, r)| dismiss_from(&mut r.mailbox, session, |_| true).then_some(run))
            .collect();
        for run in runs {
            self.habit_changed(run, cx);
        }
    }
}
/// Puts `session` in the Pilot's folder, creating it or adopting the one older versions named
/// after the Pilot.
fn file_in_pilot_folder(groups: &mut Groups, pilot: &pilot::Pilot, session: i64) {
    let folder_id = pilot_folder_id(&pilot.id);
    let label = pilot.label();
    let color = pilot::COLORS_PILOT
        .iter()
        .position(|c| *c == pilot.color)
        .map(|n| if n == 8 { 6 } else { n + 1 });
    let folder = if groups.folder(&folder_id).is_some() {
        Some(folder_id.clone())
    } else if let Some(legacy) = groups.folders.iter().find(|f| f.name == label).cloned() {
        // Older versions filed by name; move that folder to the stable id.
        let mut sessions = legacy.session_ids.clone();
        sessions.push(session);
        groups.dissolve(&legacy.id);
        groups.create_folder(&sessions, folder_id.clone())
    } else {
        groups.create_folder(&[session], folder_id.clone())
    };
    if let Some(folder) = folder {
        groups.rename_folder(&folder, &label);
        groups.set_color(&folder, color);
        groups.add_to_folder(&folder, session);
    }
}
fn new_report(entries: Vec<completions::Entry>) -> Message {
    Message {
        id: pilot::new_id(),
        text: completions::report_message(&entries),
        sent_at: Some(now_ms()),
        internal: true,
        error: None,
        attachments: Vec::new(),
        report: entries,
    }
}
/// Renders a queued report again after its sessions changed. A fresh id waits for this text
/// to be saved before it can be dispatched.
fn rewrite_report(message: &mut Message) {
    message.id = pilot::new_id();
    message.text = completions::report_message(&message.report);
    message.sent_at = Some(now_ms());
}
/// Takes `session` out of the open reports in `queue`; returns whether any changed.
fn dismiss_from(queue: &mut Vec<Message>, session: i64, open: impl Fn(&Message) -> bool) -> bool {
    let mut changed = false;
    queue.retain_mut(|m| {
        if !open(m) || !completions::dismiss(&mut m.report, session) {
            return true;
        }
        changed = true;
        if m.report.is_empty() {
            return false;
        }
        rewrite_report(m);
        true
    });
    changed
}
