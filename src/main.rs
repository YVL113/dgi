#![windows_subsystem = "windows"]

extern crate native_windows_gui as nwg;
use nwg::NativeUi;

#[derive(Default)]
pub struct DgiApp {
    window: nwg::Window,
}

impl DgiApp {
    fn on_window_close(&self) {
        nwg::stop_thread_dispatch();
    }
}

pub struct DgiAppUi {
    #[allow(dead_code)]
    inner: std::rc::Rc<DgiApp>,
    default_handler: nwg::EventHandler,
}

impl NativeUi<DgiAppUi> for DgiApp {
    fn build_ui(mut data: DgiApp) -> Result<DgiAppUi, nwg::NwgError> {
        nwg::Window::builder()
            .flags(
                nwg::WindowFlags::WINDOW
                    | nwg::WindowFlags::VISIBLE
                    | nwg::WindowFlags::MINIMIZE_BOX
                    | nwg::WindowFlags::MAXIMIZE_BOX
                    | nwg::WindowFlags::RESIZABLE,
            )
            .size((1000, 700))
            .position((300, 150))
            .title("dgi")
            .build(&mut data.window)?;

        let handle = data.window.handle;
        let inner = std::rc::Rc::new(data);
        let evt_inner = inner.clone();

        let default_handler =
            nwg::full_bind_event_handler(&handle, move |evt, _evt_data, _handle| {
                if evt == nwg::Event::OnWindowClose {
                    evt_inner.on_window_close();
                }
            });

        Ok(DgiAppUi {
            inner,
            default_handler,
        })
    }
}

impl Drop for DgiAppUi {
    fn drop(&mut self) {
        nwg::unbind_event_handler(&self.default_handler);
    }
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
}

fn main() {
    unsafe {
        // -4 is DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
        SetProcessDpiAwarenessContext(-4);
    }

    nwg::init().expect("Failed to initialize Native Windows GUI");
    let _app = DgiApp::build_ui(Default::default()).expect("Failed to build UI");
    nwg::dispatch_thread_events();
}
