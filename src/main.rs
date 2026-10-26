#![windows_subsystem = "windows"]

extern crate native_windows_gui as nwg;
use nwg::NativeUi;

#[derive(Default)]
pub struct DgiApp {
    window: nwg::Window,
    tabs: nwg::TabsContainer,
    tab_scalper: nwg::Tab,
    tab_replace: nwg::Tab,
    tab_settings: nwg::Tab,
    label_scalper: nwg::Label,
    label_replace: nwg::Label,
    label_settings: nwg::Label,
    tabs_font: nwg::Font,
    large_font: nwg::Font,
}

impl DgiApp {
    fn on_window_close(&self) {
        nwg::stop_thread_dispatch();
    }

    fn on_resize(&self) {
        let (w, h) = self.window.size();
        if w == 0 || h == 0 {
            return;
        }

        self.tabs.set_size(w, h);

        if let Some(hwnd) = self.tab_scalper.handle.hwnd() {
            let mut r: RECT = unsafe { std::mem::zeroed() };
            unsafe { GetClientRect(hwnd as _, &mut r) };
            let (tab_w, tab_h) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
            if tab_w > 0 && tab_h > 0 {
                self.label_scalper.set_size(tab_w, tab_h);
                self.label_replace.set_size(tab_w, tab_h);
                self.label_settings.set_size(tab_w, tab_h);
            }
        }
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

        nwg::Font::builder()
            .size(16)
            .family("Segoe UI")
            .build(&mut data.tabs_font)?;

        nwg::Font::builder()
            .size(54)
            .weight(600)
            .family("Segoe UI")
            .build(&mut data.large_font)?;

        let (win_w, win_h) = data.window.size();

        nwg::TabsContainer::builder()
            .parent(&data.window)
            .position((0, 0))
            .size((win_w as i32, win_h as i32))
            .font(Some(&data.tabs_font))
            .build(&mut data.tabs)?;

        nwg::Tab::builder()
            .text("Scalper")
            .parent(&data.tabs)
            .build(&mut data.tab_scalper)?;

        nwg::Tab::builder()
            .text("Replace")
            .parent(&data.tabs)
            .build(&mut data.tab_replace)?;

        nwg::Tab::builder()
            .text("Settings")
            .parent(&data.tabs)
            .build(&mut data.tab_settings)?;

        let btn_face = unsafe { GetSysColor(COLOR_BTNFACE) };
        let bg_color = [
            (btn_face & 0xFF) as u8,
            ((btn_face >> 8) & 0xFF) as u8,
            ((btn_face >> 16) & 0xFF) as u8,
        ];

        nwg::Label::builder()
            .parent(&data.tab_scalper)
            .text("Scalper")
            .position((0, 0))
            .size((win_w as i32, win_h as i32))
            .font(Some(&data.large_font))
            .background_color(Some(bg_color))
            .h_align(nwg::HTextAlign::Center)
            .v_align(nwg::VTextAlign::Center)
            .build(&mut data.label_scalper)?;

        nwg::Label::builder()
            .parent(&data.tab_replace)
            .text("Replace")
            .position((0, 0))
            .size((win_w as i32, win_h as i32))
            .font(Some(&data.large_font))
            .background_color(Some(bg_color))
            .h_align(nwg::HTextAlign::Center)
            .v_align(nwg::VTextAlign::Center)
            .build(&mut data.label_replace)?;

        nwg::Label::builder()
            .parent(&data.tab_settings)
            .text("Settings")
            .position((0, 0))
            .size((win_w as i32, win_h as i32))
            .font(Some(&data.large_font))
            .background_color(Some(bg_color))
            .h_align(nwg::HTextAlign::Center)
            .v_align(nwg::VTextAlign::Center)
            .build(&mut data.label_settings)?;

        data.on_resize();

        let handle = data.window.handle;
        let inner = std::rc::Rc::new(data);
        let evt_inner = inner.clone();

        let default_handler =
            nwg::full_bind_event_handler(&handle, move |evt, _evt_data, h| match evt {
                nwg::Event::OnWindowClose => {
                    if h == evt_inner.window.handle {
                        evt_inner.on_window_close();
                    }
                }
                nwg::Event::OnResize => {
                    if h == evt_inner.window.handle {
                        evt_inner.on_resize();
                    }
                }
                nwg::Event::TabsContainerChanged => {
                    if h == evt_inner.tabs.handle {
                        evt_inner.on_resize();
                    }
                }
                _ => {}
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

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
    fn GetSysColor(n_index: i32) -> u32;
    fn GetClientRect(hwnd: isize, rect: *mut RECT) -> i32;
}

const COLOR_BTNFACE: i32 = 15;

fn main() {
    unsafe {
        // -4 is DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
        SetProcessDpiAwarenessContext(-4);
    }

    nwg::init().expect("Failed to initialize Native Windows GUI");
    let _app = DgiApp::build_ui(Default::default()).expect("Failed to build UI");
    nwg::dispatch_thread_events();
}
