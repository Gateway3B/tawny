use dioxus::prelude::*;
use g3_route_transitions::{RouteTransitions, try_animated_back};

/// Mouse navigation must traverse the router's history, not the WebView's
/// document history. Keep the receiver in the shell so changing pages cannot
/// cancel a traversal or lose the next press.
pub(super) fn use_mouse_navigation<Route: Routable + RouteTransitions>() {
    let navigator = use_navigator();
    use_effect(move || {
        spawn(async move {
            let mut events = document::eval(include_str!("../../assets/tawny_mouse_navigation.js"));
            while let Ok(direction) = events.recv::<i8>().await {
                match direction {
                    -1 => {
                        try_animated_back::<Route>().await;
                    }
                    1 if navigator.can_go_forward() => navigator.go_forward(),
                    _ => {}
                }
            }
        });
    });
    use_drop(|| {
        let _ = document::eval("window[Symbol.for('tawny.mouse-navigation')]?.dispose();");
    });
}
