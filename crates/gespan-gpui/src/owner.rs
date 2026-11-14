//! A child view's handle on the entity that owns it.
//!
//! GPUI aborts the app ("cannot read Shell while it is already being
//! updated") when an entity is read or updated while it is already being
//! updated. A child that keeps a handle on its owner hits that whenever the
//! owner calls into the child, directly or a few calls down, and the child
//! reads its owner back: the Pilots rail did it while the Shell drew it.
//!
//! [`Owner`] is that handle. Reading or updating through it never
//! double-leases: while the owner is busy the call does nothing and returns
//! `None`. Tests panic there instead, naming the caller, so the mistake is
//! caught where it is made; the app logs it and carries on. The fix is the
//! same either way: let the owner pass the data in, or defer the work.
use gpui::{App, Context, WeakEntity};
use std::panic::Location;

pub(crate) struct Owner<T: 'static> {
    handle: WeakEntity<T>,
}

impl<T: 'static> Clone for Owner<T> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle.clone(),
        }
    }
}

impl<T: 'static> From<WeakEntity<T>> for Owner<T> {
    fn from(handle: WeakEntity<T>) -> Self {
        Self { handle }
    }
}

impl<T: 'static> Owner<T> {
    /// The raw handle, for code that runs straight from the event loop and
    /// so inside no other entity's update: spawned tasks and deferred
    /// callbacks. It has no guard; everything else reads and updates through
    /// [`Self::read`] and [`Self::update`].
    pub(crate) fn handle(&self) -> &WeakEntity<T> {
        &self.handle
    }

    /// Reads the owner; `None` once it is gone, or while it is being
    /// updated (see the module docs).
    #[track_caller]
    pub(crate) fn read<R>(&self, cx: &App, read: impl FnOnce(&T, &App) -> R) -> Option<R> {
        self.read_as(STRICT, cx, read)
    }

    #[track_caller]
    fn read_as<R>(&self, strict: bool, cx: &App, read: impl FnOnce(&T, &App) -> R) -> Option<R> {
        let owner = self.handle.upgrade()?;
        match owner.try_read(cx) {
            Some(state) => Some(read(state, cx)),
            None => busy::<T, R>(strict, Location::caller()),
        }
    }

    /// Updates the owner; `None` once it is gone, or while it is being
    /// updated (see the module docs).
    #[track_caller]
    pub(crate) fn update<R>(
        &self,
        cx: &mut App,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> Option<R> {
        self.update_as(STRICT, cx, update)
    }

    #[track_caller]
    fn update_as<R>(
        &self,
        strict: bool,
        cx: &mut App,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> Option<R> {
        let owner = self.handle.upgrade()?;
        if owner.try_read(cx).is_none() {
            return busy::<T, R>(strict, Location::caller());
        }
        Some(owner.update(cx, update))
    }
}

/// Tests fail on a re-entrant read or update; the app logs it and goes on.
const STRICT: bool = cfg!(test);

/// The owner is mid-update, so the read or update at `caller` re-enters it.
fn busy<T, R>(strict: bool, caller: &'static Location<'static>) -> Option<R> {
    let owner = std::any::type_name::<T>();
    if strict {
        panic!(
            "{caller}: {owner} is being updated, so its child cannot read or update it here; \
             have {owner} pass the data in, or defer the call"
        );
    }
    tracing::error!(
        kind = "reentrancy",
        owner,
        at = %caller,
        "skipped a read or update of an entity that is being updated"
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, Entity, TestAppContext};

    struct Parent {
        value: u32,
    }
    struct Child {
        owner: Owner<Parent>,
    }

    fn family(cx: &mut TestAppContext) -> (Entity<Parent>, Entity<Child>) {
        let parent = cx.new(|_| Parent { value: 7 });
        let owner = Owner::from(parent.downgrade());
        let child = cx.new(|_| Child { owner });
        (parent, child)
    }

    /// The crash this module exists for: a child reading its owner back
    /// while the owner is calling into it, as the Pilots rail did while
    /// the Shell drew it.
    #[gpui::test]
    #[should_panic(expected = "while it is already being updated")]
    fn a_raw_read_of_a_busy_owner_aborts(cx: &mut TestAppContext) {
        let (parent, child) = family(cx);
        parent.update(cx, |_, cx| {
            child.update(cx, |child, cx| {
                let owner = child.owner.handle().upgrade().unwrap();
                owner.read(cx).value
            })
        });
    }

    #[gpui::test]
    #[should_panic(expected = "Parent is being updated, so its child cannot read or update it")]
    fn tests_fail_on_a_read_of_a_busy_owner(cx: &mut TestAppContext) {
        let (parent, child) = family(cx);
        parent.update(cx, |_, cx| {
            child.update(cx, |child, cx| {
                child.owner.read(cx, |parent, _| parent.value)
            })
        });
    }

    // The caller's file and line; Windows separates the path with a backslash.
    #[gpui::test]
    #[should_panic(expected = "owner.rs:")]
    fn the_failure_names_the_reading_line(cx: &mut TestAppContext) {
        let (parent, child) = family(cx);
        parent.update(cx, |_, cx| {
            child.update(cx, |child, cx| {
                child.owner.update(cx, |parent, _| parent.value += 1)
            })
        });
    }

    /// Outside tests a re-entrant read or update is skipped, not a crash.
    #[gpui::test]
    fn the_app_skips_a_busy_owner_and_leaves_it_as_it_was(cx: &mut TestAppContext) {
        let (parent, child) = family(cx);
        let (read, updated) = parent.update(cx, |_, cx| {
            child.update(cx, |child, cx| {
                let read = child.owner.read_as(false, cx, |parent, _| parent.value);
                let updated = child
                    .owner
                    .update_as(false, cx, |parent, _| parent.value = 99);
                (read, updated)
            })
        });
        assert_eq!(read, None);
        assert_eq!(updated, None);
        assert_eq!(parent.read_with(cx, |parent, _| parent.value), 7);
    }

    #[gpui::test]
    fn an_idle_owner_reads_and_updates(cx: &mut TestAppContext) {
        let (parent, child) = family(cx);
        child.update(cx, |child, cx| {
            assert_eq!(child.owner.read(cx, |parent, _| parent.value), Some(7));
            assert_eq!(
                child.owner.update(cx, |parent, _| {
                    parent.value += 1;
                    parent.value
                }),
                Some(8)
            );
        });
        assert_eq!(parent.read_with(cx, |parent, _| parent.value), 8);
    }

    #[gpui::test]
    fn a_dropped_owner_reads_as_none(cx: &mut TestAppContext) {
        let (parent, child) = family(cx);
        drop(parent);
        cx.run_until_parked();
        child.update(cx, |child, cx| {
            assert_eq!(child.owner.read(cx, |parent, _| parent.value), None);
            assert_eq!(child.owner.update(cx, |parent, _| parent.value), None);
        });
    }

    /// Every struct that keeps the Shell holds it as an `Owner`, so no child
    /// can read the Shell back without the guard.
    #[test]
    fn no_struct_keeps_a_raw_weak_shell() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut dirs = vec![src.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for line in raw_weak_shell_fields(&text) {
                        let file = path.strip_prefix(&src).unwrap().display();
                        found.push(format!("src/{file}:{line}"));
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "keep the Shell as crate::owner::Owner<Shell>, not WeakEntity<Shell>: {found:?}"
        );
    }

    /// 1-based lines of struct fields typed `WeakEntity<Shell>`.
    fn raw_weak_shell_fields(text: &str) -> Vec<usize> {
        let mut found = Vec::new();
        let mut closing: Option<String> = None;
        for (ix, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            let indent = &line[..line.len() - trimmed.len()];
            match &closing {
                Some(end) if line == end => closing = None,
                Some(_) => {
                    let ty = trimmed.split_once(':').map(|(_, ty)| ty.replace(' ', ""));
                    if ty.is_some_and(|ty| {
                        ty.trim_end_matches(',')
                            .trim_start_matches("gpui::")
                            .strip_prefix("WeakEntity<")
                            .is_some_and(|rest| {
                                let rest = rest
                                    .trim_start_matches("crate::shell::")
                                    .trim_start_matches("super::");
                                rest == "Shell>"
                            })
                    }) {
                        found.push(ix + 1);
                    }
                }
                None => {
                    let decl = trimmed
                        .trim_start_matches("pub(crate) ")
                        .trim_start_matches("pub(super) ")
                        .trim_start_matches("pub ");
                    if decl.starts_with("struct ") && trimmed.ends_with('{') {
                        closing = Some(format!("{indent}}}"));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn the_scan_finds_a_raw_weak_shell_field() {
        let text = "pub(crate) struct Rail {\n    id: u32,\n    owner: gpui::WeakEntity<Shell>,\n    fine: Owner<Shell>,\n}\nfn f(owner: WeakEntity<Shell>) {}\nstruct B {\n    shell: WeakEntity<crate::shell::Shell>,\n}\n";
        assert_eq!(raw_weak_shell_fields(text), vec![3, 8]);
    }
}
