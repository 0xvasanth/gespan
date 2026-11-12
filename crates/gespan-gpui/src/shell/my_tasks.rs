//! The shell's side of My tasks: the page entity, its count for the sidebar,
//! and opening it.
use super::*;
use crate::my_tasks::{MyTasks, TaskItem};

impl Shell {
    /// The page, created the first time anything needs it.
    pub(super) fn ensure_my_tasks(&mut self, cx: &mut Context<Self>) -> Entity<MyTasks> {
        if let Some(view) = &self.my_tasks {
            return view.clone();
        }
        let view = cx.new(MyTasks::new);
        self.my_tasks = Some(view.clone());
        view
    }

    /// Open tasks, for the sidebar count.
    pub(super) fn my_task_count(&self, cx: &App) -> usize {
        self.my_tasks
            .as_ref()
            .map_or(0, |view| view.read(cx).open_count())
    }

    /// Hands the page a fresh snapshot and redraws the sidebar count when it
    /// changes.
    pub(super) fn set_my_tasks(&mut self, tasks: Vec<TaskItem>, cx: &mut Context<Self>) {
        let before = self.my_task_count(cx);
        let view = self.ensure_my_tasks(cx);
        let now = gespan_core::time::now_ms();
        view.update(cx, |view, cx| view.set_tasks(tasks, now, cx));
        if self.my_task_count(cx) != before {
            self.notify_navigation(cx);
        }
    }

    pub(super) fn open_my_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_app_view(window, cx);
        self.ensure_my_tasks(cx);
        self.show_app_view(app_views::AppView::MyTasks, window, cx);
    }

    #[cfg(test)]
    pub(super) fn show_my_tasks_for_test(&mut self, tasks: Vec<TaskItem>, cx: &mut Context<Self>) {
        self.set_my_tasks(tasks, cx);
    }
}
