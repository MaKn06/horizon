//! xD Client: one window with a web view that shows the launcher's pages (`ui/index.html`). The
//! page asks for everything through `window.ipc` (`backend.rs`); installing, signing in and
//! playing go through the Horizon launcher's command line, shipped next to this program.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

/// What the backend's threads ask the window to do.
pub enum UserEvent {
    /// Run this script in the page (answers and events).
    Eval(String),
    /// Move the window with the mouse (the top bar was pressed).
    Drag,
    Hide,
    Show,
    Quit,
}

fn main() {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    #[allow(unused_mut)]
    let mut builder = WindowBuilder::new()
        .with_title("xD Client")
        .with_inner_size(LogicalSize::new(1220.0, 800.0))
        .with_min_inner_size(LogicalSize::new(940.0, 640.0));
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder = builder
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true);
    }
    let window = builder.build(&event_loop).expect("the window");

    let ipc_proxy = proxy.clone();
    let web = WebViewBuilder::new()
        .with_html(include_str!("../ui/index.html"))
        .with_background_color((7, 8, 12, 255))
        .with_devtools(cfg!(debug_assertions))
        .with_initialization_script(&format!(
            "window.__xdPlatform = {:?};",
            std::env::consts::OS
        ))
        .with_custom_protocol("xdfile".into(), |_id, req| backend::local_file(req))
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            backend::handle(req.into_body(), ipc_proxy.clone())
        });

    #[cfg(not(target_os = "linux"))]
    let webview = web.build(&window).expect("the web view");
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        web.build_gtk(window.default_vbox().expect("a GTK box"))
            .expect("the web view")
    };

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Eval(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::UserEvent(UserEvent::Drag) => {
                let _ = window.drag_window();
            }
            Event::UserEvent(UserEvent::Hide) => window.set_visible(false),
            Event::UserEvent(UserEvent::Show) => {
                window.set_visible(true);
                window.set_focus();
            }
            Event::UserEvent(UserEvent::Quit)
            | Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
