//! Blur radius: the desktop blur behind a glass window. Only macOS lets us set
//! it, on the Gaussian filter of the visual-effect view gpui puts behind the
//! window. Windows Acrylic and Linux compositors choose their own.
use gpui::{Window, WindowAppearance};

pub(crate) const BLUR_ADJUSTABLE: bool = cfg!(target_os = "macos");

/// What the filter was last set for. AppKit rebuilds its private layers when
/// the window's appearance or focus changes, so each change sets it again.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Applied {
    radius: u32,
    appearance: WindowAppearance,
    active: bool,
}

pub(crate) fn sync(applied: &mut Option<Applied>, radius: u32, window: &mut Window) {
    let next = Applied {
        radius,
        appearance: window.appearance(),
        active: window.is_window_active(),
    };
    if !BLUR_ADJUSTABLE || *applied == Some(next) {
        return;
    }
    *applied = Some(next);
    #[cfg(target_os = "macos")]
    apply(radius, FRAMES_TO_WAIT, window);
}

/// AppKit builds the filter some frames after gpui adds the view.
#[cfg(target_os = "macos")]
const FRAMES_TO_WAIT: u32 = 120;

#[cfg(target_os = "macos")]
fn apply(radius: u32, frames: u32, window: &mut Window) {
    window.on_next_frame(move |window, _| {
        if !set_radius(window, radius) && frames > 0 {
            apply(radius, frames - 1, window);
        }
    });
}

/// True once a blur filter has the radius.
#[cfg(target_os = "macos")]
fn set_radius(window: &Window, radius: u32) -> bool {
    use objc2::{msg_send, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::NSView;
    let Some(content) = crate::traffic_lights::ns_window(window).and_then(|w| w.contentView())
    else {
        return false;
    };
    let mut found = false;
    for view in content.subviews().iter() {
        let view: &NSView = &view;
        if view.class().name().to_bytes() != b"BlurredView" {
            continue;
        }
        let layer: Option<Retained<AnyObject>> = unsafe { msg_send![view, layer] };
        if let Some(layer) = layer {
            // These filters are AppKit's own; an OS change must not let an
            // Objective-C exception escape into the frame callback.
            let result = objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
                set_layer_radius(&layer, f64::from(radius))
            }));
            match result {
                Ok(set) => found |= set,
                Err(error) => {
                    tracing::warn!(?error, "setting the window blur radius failed");
                    return true;
                }
            }
        }
    }
    found
}

/// Replaces rather than edits a filter: the installed one is shared with Core
/// Animation's committed render tree, so changing it in place never shows.
#[cfg(target_os = "macos")]
fn set_layer_radius(layer: &objc2::runtime::AnyObject, radius: f64) -> bool {
    use objc2::{msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSArray, NSNumber, NSString};
    let key = NSString::from_str("inputRadius");
    let mut found = false;
    let filters: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![layer, filters] };
    if let Some(filters) = filters {
        let mut changed = false;
        let replaced: Vec<Retained<AnyObject>> = filters
            .iter()
            .map(|filter| {
                let description: Retained<NSString> = unsafe { msg_send![&*filter, description] };
                if !description.to_string().contains("Blur") {
                    return filter;
                }
                found = true;
                let current: Option<Retained<NSNumber>> =
                    unsafe { msg_send![&*filter, valueForKey: &*key] };
                if current.is_some_and(|value| value.doubleValue() == radius) {
                    return filter;
                }
                changed = true;
                let copy: Retained<AnyObject> = unsafe { msg_send![&*filter, copy] };
                let value = NSNumber::numberWithDouble(radius);
                let _: () = unsafe { msg_send![&*copy, setValue: &*value, forKey: &*key] };
                copy
            })
            .collect();
        if changed {
            let filters = NSArray::from_retained_slice(&replaced);
            let _: () = unsafe { msg_send![layer, setFilters: &*filters] };
        }
    }
    let sublayers: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![layer, sublayers] };
    for child in sublayers.iter().flat_map(|children| children.iter()) {
        found |= set_layer_radius(&child, radius);
    }
    found
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use objc2::{msg_send, rc::Retained, runtime::AnyClass, runtime::AnyObject};
    use objc2_foundation::{NSArray, NSNumber, NSString};

    fn filter(kind: &str) -> Retained<AnyObject> {
        let class = AnyClass::get(c"CAFilter").expect("Core Animation provides filters");
        unsafe { msg_send![class, filterWithType: &*NSString::from_str(kind)] }
    }
    fn filters(layer: &AnyObject) -> Retained<NSArray<AnyObject>> {
        unsafe { msg_send![layer, filters] }
    }
    fn radius(filter: &AnyObject) -> Option<f64> {
        let key = NSString::from_str("inputRadius");
        let value: Option<Retained<NSNumber>> = unsafe { msg_send![filter, valueForKey: &*key] };
        value.map(|value| value.doubleValue())
    }

    #[test]
    fn radius_replaces_the_gaussian_filter_on_nested_layers_and_keeps_the_rest() {
        let result = objc2::exception::catch(|| {
            let class = AnyClass::get(c"CALayer").expect("Core Animation provides layers");
            let root: Retained<AnyObject> = unsafe { msg_send![class, new] };
            let child: Retained<AnyObject> = unsafe { msg_send![class, new] };
            let _: () = unsafe { msg_send![&*root, addSublayer: &*child] };
            let installed =
                NSArray::from_retained_slice(&[filter("gaussianBlur"), filter("colorSaturate")]);
            let _: () = unsafe { msg_send![&*child, setFilters: &*installed] };

            let empty: Retained<AnyObject> = unsafe { msg_send![class, new] };
            assert!(!set_layer_radius(&empty, 24.), "no filter yet: try again");

            for value in [60., 1., 64., 24.] {
                let before = filters(&child).objectAtIndex(0);
                let was = radius(&before);
                assert!(set_layer_radius(&root, value));
                let after = filters(&child);
                assert_eq!(after.len(), 2);
                let blur = after.objectAtIndex(0);
                assert_eq!(radius(&blur), Some(value));
                assert!(
                    !std::ptr::eq(&*before, &*blur),
                    "a new radius must install a new filter to reach the render tree"
                );
                assert_eq!(radius(&before), was);

                assert!(set_layer_radius(&root, value));
                assert!(
                    std::ptr::eq(&*blur, &*filters(&child).objectAtIndex(0)),
                    "an unchanged radius leaves the installed filter alone"
                );
            }
        });
        result.unwrap_or_else(|exception| {
            panic!("Core Animation rejected the blur filter: {exception:?}")
        });
    }
}
