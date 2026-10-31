//! Several workspace windows, after `src-tauri/src/window.rs`.
//!
//! Each window is its own `Shell` over the same store. The first one, the
//! primary, also runs what the app must run once: Pilots, reminders,
//! automations, flows, the mobile host, the app bridge, the Quick Composer
//! hotkey, the tray and persistence. So it lives as long as the app does:
//! closing it hides it, and it comes back for whatever needs a window.
//!
//! A chat lives in one window at a time, because its `Conversation` holds the
//! agent. Another window that opens it shows it where it runs, and closing a
//! further window hands its running chats to the primary.
use crate::shell::Shell;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, EntityId, Global, Pixels,
    TitlebarOptions, Window, WindowBounds, WindowDecorations, WindowHandle, WindowOptions, point,
    px, size,
};
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

/// How far each new window sits from the last, so it does not hide it.
const CASCADE: f32 = 28.;
const CASCADE_STEPS: usize = 8;

#[derive(Debug)]
struct Entry<K> {
    key: K,
    hidden: bool,
}

/// Which windows are open and what each one holds, apart from GPUI.
#[derive(Debug)]
pub(crate) struct Registry<K> {
    /// The primary first.
    windows: Vec<Entry<K>>,
    owners: HashMap<i64, K>,
    running: HashMap<K, Vec<i64>>,
}

impl<K> Default for Registry<K> {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            owners: HashMap::new(),
            running: HashMap::new(),
        }
    }
}

impl<K: Copy + Eq + Hash> Registry<K> {
    pub fn add(&mut self, key: K, secondary: bool) {
        if self.windows.iter().any(|entry| entry.key == key) {
            return;
        }
        let entry = Entry { key, hidden: false };
        if secondary {
            self.windows.push(entry);
        } else {
            self.windows.insert(0, entry);
        }
    }
    /// Forgets a window with the chats it held, which are free to open again.
    pub fn remove(&mut self, key: K) {
        self.windows.retain(|entry| entry.key != key);
        self.owners.retain(|_, owner| *owner != key);
        self.running.remove(&key);
    }
    pub fn primary(&self) -> Option<K> {
        self.windows.first().map(|entry| entry.key)
    }
    pub fn len(&self) -> usize {
        self.windows.len()
    }
    pub fn keys(&self) -> impl Iterator<Item = K> + '_ {
        self.windows.iter().map(|entry| entry.key)
    }
    /// Gives a chat to a window, unless another already holds it.
    pub fn claim(&mut self, session: i64, key: K) -> bool {
        *self.owners.entry(session).or_insert(key) == key
    }
    pub fn release(&mut self, session: i64, key: K) {
        if self.owners.get(&session) == Some(&key) {
            self.owners.remove(&session);
        }
    }
    pub fn owner(&self, session: i64) -> Option<K> {
        self.owners.get(&session).copied()
    }
    pub fn set_hidden(&mut self, key: K, hidden: bool) {
        if let Some(entry) = self.windows.iter_mut().find(|entry| entry.key == key) {
            entry.hidden = hidden;
        }
    }
    pub fn hidden(&self) -> Vec<K> {
        let hidden = self.windows.iter().filter(|entry| entry.hidden);
        hidden.map(|entry| entry.key).collect()
    }
    pub fn visible_elsewhere(&self, key: K) -> usize {
        let visible = self.windows.iter().filter(|entry| !entry.hidden);
        visible.filter(|entry| entry.key != key).count()
    }
    /// Records one window's running chats and returns every window's, without
    /// repeats: the store keeps a single list for the next launch to resume.
    pub fn set_running(&mut self, key: K, chats: Vec<i64>) -> Vec<i64> {
        self.running.insert(key, chats);
        let mut seen = HashSet::new();
        let mut all = Vec::new();
        for window in self.keys() {
            let chats = self.running.get(&window).into_iter().flatten();
            all.extend(chats.filter(|id| seen.insert(**id)));
        }
        // A window not drawn yet has no entry, but its chats still count.
        let own = self.running.get(&key).into_iter().flatten();
        all.extend(own.filter(|id| seen.insert(**id)));
        all
    }
    pub fn running_elsewhere(&self, key: K) -> usize {
        let others = self.running.iter().filter(|(window, _)| **window != key);
        others.map(|(_, chats)| chats.len()).sum()
    }
    pub fn is_running(&self, session: i64) -> bool {
        self.running.values().any(|chats| chats.contains(&session))
    }
}

/// `bounds` moved down and right for the `opened`th new window, wrapping so
/// the windows never walk off the screen.
fn cascaded(bounds: WindowBounds, opened: usize) -> WindowBounds {
    let step = px(CASCADE * ((opened % CASCADE_STEPS) + 1) as f32);
    let moved = |bounds: Bounds<Pixels>| Bounds {
        origin: point(bounds.origin.x + step, bounds.origin.y + step),
        size: bounds.size,
    };
    match bounds {
        WindowBounds::Windowed(bounds) => WindowBounds::Windowed(moved(bounds)),
        // A new window beside a full-screen one is an ordinary one.
        WindowBounds::Maximized(bounds) | WindowBounds::Fullscreen(bounds) => {
            WindowBounds::Windowed(moved(bounds))
        }
    }
}

#[derive(Default)]
struct Windows {
    registry: Registry<EntityId>,
    handles: HashMap<EntityId, WindowHandle<Shell>>,
    /// The workspace settings every window last agreed on.
    shared: Option<String>,
    opened: usize,
}
impl Global for Windows {}

fn windows_mut(cx: &mut App) -> Option<&mut Windows> {
    cx.has_global::<Windows>()
        .then(|| cx.global_mut::<Windows>())
}

fn registry(cx: &App) -> Option<&Registry<EntityId>> {
    cx.try_global::<Windows>().map(|windows| &windows.registry)
}

fn handle(key: EntityId, cx: &App) -> Option<WindowHandle<Shell>> {
    cx.try_global::<Windows>()?.handles.get(&key).copied()
}

pub(crate) fn handles_elsewhere(key: EntityId, cx: &App) -> Vec<WindowHandle<Shell>> {
    let Some(windows) = cx.try_global::<Windows>() else {
        return Vec::new();
    };
    windows
        .registry
        .keys()
        .filter(|window| *window != key)
        .filter_map(|window| windows.handles.get(&window).copied())
        .collect()
}

/// Watches windows close; the app ends with its last workspace window, even
/// if a floating Pilot or a hidden Quick Composer is left.
pub(crate) fn install(cx: &mut App) {
    cx.on_window_closed(|cx, _| {
        if forget_closed(cx) {
            crate::quit(cx);
        }
    })
    .detach();
}

/// Returns whether no workspace window is left.
pub(crate) fn forget_closed(cx: &mut App) -> bool {
    let open = cx.windows();
    let gone: Vec<EntityId> = cx
        .try_global::<Windows>()
        .into_iter()
        .flat_map(|windows| &windows.handles)
        .filter(|(_, handle)| !open.contains(&AnyWindowHandle::from(**handle)))
        .map(|(key, _)| *key)
        .collect();
    for key in gone {
        remove(key, cx);
    }
    !open
        .iter()
        .any(|window| window.downcast::<Shell>().is_some())
}

/// A primary begins a new set of windows; session ids mean nothing across
/// stores. The app makes one, and tests make one for each store they open.
pub(crate) fn start(cx: &mut App) {
    cx.set_global(Windows::default());
}

pub(crate) fn register(
    shell: &Entity<Shell>,
    window: AnyWindowHandle,
    secondary: bool,
    cx: &mut App,
) {
    let Some(handle) = window.downcast::<Shell>() else {
        return;
    };
    let windows = cx.default_global::<Windows>();
    windows.registry.add(shell.entity_id(), secondary);
    windows.handles.insert(shell.entity_id(), handle);
}

pub(crate) fn remove(key: EntityId, cx: &mut App) {
    if let Some(windows) = windows_mut(cx) {
        windows.registry.remove(key);
        windows.handles.remove(&key);
    }
}

pub(crate) fn primary(cx: &App) -> Option<WindowHandle<Shell>> {
    handle(registry(cx)?.primary()?, cx)
}

pub(crate) fn count(cx: &App) -> usize {
    registry(cx).map_or(0, Registry::len)
}

pub(crate) fn claim(session: i64, key: EntityId, cx: &mut App) -> bool {
    cx.default_global::<Windows>().registry.claim(session, key)
}

pub(crate) fn release(session: i64, key: EntityId, cx: &mut App) {
    if let Some(windows) = windows_mut(cx) {
        windows.registry.release(session, key);
    }
}

pub(crate) fn owned_elsewhere(session: i64, key: EntityId, cx: &App) -> bool {
    registry(cx)
        .and_then(|registry| registry.owner(session))
        .is_some_and(|owner| owner != key)
}

pub(crate) fn set_hidden(key: EntityId, hidden: bool, cx: &mut App) {
    if let Some(windows) = windows_mut(cx) {
        windows.registry.set_hidden(key, hidden);
    }
}

#[cfg(test)]
pub(crate) fn is_hidden(key: EntityId, cx: &App) -> bool {
    registry(cx).is_some_and(|registry| registry.hidden().contains(&key))
}

pub(crate) fn visible_elsewhere(key: EntityId, cx: &App) -> usize {
    registry(cx).map_or(0, |registry| registry.visible_elsewhere(key))
}

pub(crate) fn set_running(key: EntityId, chats: Vec<i64>, cx: &mut App) -> Vec<i64> {
    cx.default_global::<Windows>()
        .registry
        .set_running(key, chats)
}

pub(crate) fn running_elsewhere(key: EntityId, cx: &App) -> usize {
    registry(cx).map_or(0, |registry| registry.running_elsewhere(key))
}

pub(crate) fn is_running(session: i64, cx: &App) -> bool {
    registry(cx).is_some_and(|registry| registry.is_running(session))
}

pub(crate) fn unsaved_elsewhere(key: EntityId, cx: &App) -> bool {
    handles_elsewhere(key, cx)
        .into_iter()
        .any(|window| window.read(cx).is_ok_and(|shell| shell.unsaved_files(cx)))
}

pub(crate) fn conversations_elsewhere(
    key: EntityId,
    cx: &App,
) -> Vec<Entity<crate::conversation::Conversation>> {
    handles_elsewhere(key, cx)
        .into_iter()
        .filter_map(|window| window.read(cx).ok().map(Shell::live_chats))
        .flatten()
        .collect()
}

/// Runs `action` on the primary once the current update has ended.
pub(crate) fn on_primary(
    cx: &mut App,
    action: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) {
    if let Some(window) = primary(cx) {
        run_on(window, cx, action);
    }
}

fn run_on(
    window: WindowHandle<Shell>,
    cx: &mut App,
    action: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) {
    cx.defer(move |cx| {
        let _ = window.update(cx, action);
    });
}

/// Runs `action` on the window holding `session`, if that is another window,
/// and returns whether it did.
pub(crate) fn on_owner(
    session: i64,
    key: EntityId,
    cx: &mut App,
    action: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) -> bool {
    if !owned_elsewhere(session, key, cx) {
        return false;
    }
    let owner = registry(cx).and_then(|registry| registry.owner(session));
    let Some(window) = owner.and_then(|owner| handle(owner, cx)) else {
        return false;
    };
    run_on(window, cx, action);
    true
}

/// Brings up the chat in the window that holds it, if that is another window.
pub(crate) fn show_elsewhere(session: i64, key: EntityId, cx: &mut App) -> bool {
    on_owner(session, key, cx, move |shell, window, cx| {
        shell.show_session(session, window, cx)
    })
}

/// Tells the other windows that sessions changed.
pub(crate) fn sessions_changed(key: EntityId, cx: &mut App) {
    if count(cx) < 2 {
        return;
    }
    for window in handles_elsewhere(key, cx) {
        run_on(window, cx, |shell, _, cx| shell.schedule_session_sync(cx));
    }
}

/// Records the workspace settings one window has and returns the windows to
/// tell, which is none if they already agree.
pub(crate) fn share(key: EntityId, shared: String, cx: &mut App) -> Vec<WindowHandle<Shell>> {
    let windows = cx.default_global::<Windows>();
    if windows.shared.as_ref() == Some(&shared) {
        return Vec::new();
    }
    windows.shared = Some(shared);
    handles_elsewhere(key, cx)
}

/// Reminders, Quick Composer and Pilot chat: a window that is needed comes
/// back if a close hid it. App-wide work keeps the primary alive, so there is
/// always one to bring back.
pub(crate) fn show_primary(cx: &mut App) {
    on_primary(cx, |shell, window, cx| shell.show_window(window, cx));
}

/// Dock icon, second launch: shows the windows a close hid, or else the one
/// last in front.
pub(crate) fn reveal(cx: &mut App) {
    let hidden: Vec<_> = registry(cx)
        .map(Registry::hidden)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|key| handle(key, cx))
        .collect();
    let front = || {
        let stack = cx.window_stack().unwrap_or_default();
        stack
            .into_iter()
            .chain(cx.windows())
            .find_map(|window| window.downcast::<Shell>())
    };
    let targets = if hidden.is_empty() {
        front().into_iter().collect()
    } else {
        hidden
    };
    for window in targets {
        let _ = window.update(cx, |shell, window, cx| shell.show_window(window, cx));
    }
}

/// File → New Window, its shortcut and the Dock menu. With the primary hidden
/// it comes back rather than leaving a window nobody can reach.
pub(crate) fn open_new_window(cx: &mut App) {
    let Some(primary) = primary(cx) else {
        return;
    };
    let primary_key = registry(cx).and_then(Registry::primary);
    if registry(cx)
        .is_some_and(|registry| primary_key.is_some_and(|key| registry.hidden().contains(&key)))
    {
        let _ = primary.update(cx, |shell, window, cx| shell.show_window(window, cx));
        return;
    }
    let Ok(seed) = primary.read(cx).map(|shell| shell.new_window_seed(cx)) else {
        return;
    };
    let windows = cx.default_global::<Windows>();
    let opened = windows.opened;
    windows.opened += 1;
    cx.spawn(async move |cx| {
        let seed = seed.load(cx).await;
        cx.update(|cx| open(seed, opened, cx));
    })
    .detach();
}

fn open(seed: crate::shell::WindowSeed, opened: usize, cx: &mut App) {
    let bounds = cascaded(seed.bounds(cx), opened);
    let options = options(bounds, seed.decorations(cx));
    let opened = cx.open_window(options, |window, cx| {
        let shell = cx.new(|cx| Shell::for_new_window(seed, cx));
        shell.update(cx, |shell, cx| shell.focus_active(window, cx));
        prepare(&shell, window, cx);
        window.activate_window();
        shell
    });
    if let Err(error) = opened {
        tracing::warn!(%error, "cannot open a new window");
    }
}

/// Every workspace window's options, so they all look the same.
pub(crate) fn options(
    bounds: WindowBounds,
    decorations: Option<WindowDecorations>,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(size(px(820.), px(520.))),
        window_decorations: decorations,
        // Linux desktops match this to the .desktop file's
        // StartupWMClass to show the app icon.
        app_id: Some("dev.gespan.native".into()),
        window_background: crate::theme::window_backdrop(true),
        // GPUI owns the custom chrome. AppKit must not reinterpret
        // its controls as native titlebar double-click/drag gestures.
        app_owns_titlebar_drag: cfg!(target_os = "macos"),
        titlebar: Some(TitlebarOptions {
            title: Some("Gespan Native".into()),
            // Windows also drops its caption: the app's drag areas
            // hit-test as the caption and the frame still resizes.
            appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
            traffic_light_position: if cfg!(target_os = "macos") {
                // Original macOS chrome: 12 pt left inset, with
                // 14 pt native buttons centered in the 40 pt bar.
                Some(point(
                    px(12.),
                    px((crate::theme::TITLE_BAR_HEIGHT - 14.) / 2.),
                ))
            } else {
                None
            },
        }),
        ..Default::default()
    }
}

/// Native window chrome, once a workspace window has its shell.
#[cfg_attr(any(test, not(target_os = "macos")), allow(unused_variables))]
pub(crate) fn prepare(shell: &Entity<Shell>, window: &mut Window, cx: &mut App) {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        use crate::{theme::TITLE_BAR_HEIGHT, traffic_lights};
        traffic_lights::pin(window, TITLE_BAR_HEIGHT);
        // AppKit rebuilds the buttons on some titlebar changes.
        shell.update(cx, |_, cx| {
            cx.observe_window_activation(window, |_, window, _| {
                traffic_lights::pin(window, TITLE_BAR_HEIGHT)
            })
            .detach();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_windows() -> Registry<u8> {
        let mut registry = Registry::default();
        registry.add(1, false);
        registry.add(2, true);
        registry
    }

    #[test]
    fn the_first_window_stays_primary_whatever_order_they_register_in() {
        let mut registry = Registry::default();
        registry.add(2, true);
        registry.add(1, false);
        registry.add(1, false);
        assert_eq!(registry.primary(), Some(1));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn a_chat_belongs_to_one_window_until_it_lets_go() {
        let mut registry = two_windows();
        assert!(registry.claim(7, 1));
        assert!(registry.claim(7, 1), "claiming again is harmless");
        assert!(!registry.claim(7, 2));
        registry.release(7, 2);
        assert_eq!(registry.owner(7), Some(1), "only the owner can release");
        registry.release(7, 1);
        assert!(registry.claim(7, 2));
    }

    #[test]
    fn closing_a_window_frees_its_chats_and_forgets_their_running_state() {
        let mut registry = two_windows();
        registry.claim(7, 2);
        registry.claim(8, 1);
        registry.set_running(2, vec![7]);
        registry.remove(2);
        assert_eq!(registry.owner(7), None);
        assert_eq!(registry.owner(8), Some(1));
        assert_eq!(registry.running_elsewhere(1), 0);
        assert!(!registry.is_running(7));
        assert!(registry.claim(7, 1));
    }

    #[test]
    fn hidden_windows_do_not_count_as_a_way_back() {
        let mut registry = two_windows();
        assert_eq!(registry.visible_elsewhere(1), 1);
        registry.set_hidden(2, true);
        assert_eq!(registry.visible_elsewhere(1), 0);
        assert_eq!(registry.hidden(), vec![2]);
        registry.set_hidden(2, false);
        assert_eq!(registry.visible_elsewhere(1), 1);
        assert!(registry.hidden().is_empty());
    }

    #[test]
    fn the_saved_running_list_covers_every_window_without_repeats() {
        let mut registry = two_windows();
        assert_eq!(registry.set_running(2, vec![5, 6]), vec![5, 6]);
        assert_eq!(registry.set_running(1, vec![6, 3]), vec![6, 3, 5]);
        assert_eq!(registry.running_elsewhere(2), 2);
        assert_eq!(registry.set_running(2, Vec::new()), vec![6, 3]);
    }

    #[test]
    fn a_window_not_yet_drawn_still_reports_its_running_chats() {
        let mut registry = Registry::default();
        assert_eq!(registry.set_running(9_u8, vec![4]), vec![4]);
    }

    #[test]
    fn new_windows_cascade_and_wrap_instead_of_stacking_exactly() {
        let base = Bounds::new(point(px(100.), px(50.)), size(px(900.), px(600.)));
        let origin = |opened| {
            cascaded(WindowBounds::Windowed(base), opened)
                .get_bounds()
                .origin
        };
        assert_eq!(origin(0), point(px(128.), px(78.)));
        assert_ne!(origin(1), origin(0));
        assert_eq!(origin(CASCADE_STEPS), origin(0));
        assert_eq!(
            cascaded(WindowBounds::Windowed(base), 3).get_bounds().size,
            base.size
        );
    }

    #[test]
    fn a_new_window_beside_a_full_screen_one_is_an_ordinary_window() {
        let base = Bounds::new(point(px(0.), px(0.)), size(px(900.), px(600.)));
        for bounds in [
            WindowBounds::Fullscreen(base),
            WindowBounds::Maximized(base),
        ] {
            assert!(matches!(cascaded(bounds, 0), WindowBounds::Windowed(_)));
        }
    }
}
