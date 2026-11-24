//! Host windows for plugin GUIs, on the CLAP main thread.
//!
//! On Windows and Linux (X11) the main thread runs a `winit` event loop when it can open a
//! display; plugin GUIs embed in windows it creates. Without a display (or on other platforms)
//! the thread runs the headless loop and there is no window system.

use crate::main_thread::Job;
use std::sync::mpsc::{Receiver, SyncSender};

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    use crate::main_thread::{MainState, SERVICE_INTERVAL};
    use clack_extensions::gui::{GuiSize, Window as ClapWindow};
    use std::ffi::{c_ulong, c_void};
    use std::time::Instant;
    use winit::application::ApplicationHandler;
    use winit::dpi::PhysicalSize;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::{Window, WindowId};

    pub(crate) type WindowKey = WindowId;

    /// The running window system, lent to main-thread jobs.
    pub(crate) struct WindowSystem<'a> {
        event_loop: &'a ActiveEventLoop,
    }

    impl WindowSystem<'_> {
        pub(crate) fn create_window(
            &self,
            title: &str,
            size: GuiSize,
            resizable: bool,
        ) -> Result<HostWindow, String> {
            let attributes = Window::default_attributes()
                .with_title(title)
                .with_inner_size(PhysicalSize::new(size.width, size.height))
                .with_resizable(resizable);
            self.event_loop
                .create_window(attributes)
                .map(|window| HostWindow { window })
                .map_err(|e| e.to_string())
        }
    }

    /// A host window a plugin GUI is embedded in. Dropping it closes it.
    pub(crate) struct HostWindow {
        window: Window,
    }

    impl HostWindow {
        pub(crate) fn key(&self) -> WindowKey {
            self.window.id()
        }

        /// The window as a CLAP parent, when the platform handle is one CLAP knows.
        pub(crate) fn clap_parent(&self) -> Option<ClapWindow<'static, 'static>> {
            match self.window.window_handle().ok()?.as_raw() {
                // SAFETY: the handle is this live window's; the editor destroys the plugin GUI
                // before dropping the window.
                RawWindowHandle::Win32(handle) => {
                    Some(unsafe { ClapWindow::from_win32_hwnd(handle.hwnd.get() as *mut c_void) })
                }
                RawWindowHandle::Xlib(handle) => {
                    Some(ClapWindow::from_x11_handle(handle.window as c_ulong))
                }
                RawWindowHandle::Xcb(handle) => {
                    Some(ClapWindow::from_x11_handle(handle.window.get() as c_ulong))
                }
                _ => None,
            }
        }

        pub(crate) fn set_size(&self, size: GuiSize) {
            let _ = self
                .window
                .request_inner_size(PhysicalSize::new(size.width, size.height));
        }

        pub(crate) fn focus(&self) {
            self.window.focus_window();
        }
    }

    /// Wakes the event loop to run queued jobs.
    pub(crate) struct Waker(EventLoopProxy<()>);

    impl Waker {
        pub(crate) fn wake(&self) {
            let _ = self.0.send_event(());
        }
    }

    struct App {
        state: MainState,
        jobs: Receiver<Job>,
    }

    impl App {
        fn drain(&mut self, event_loop: &ActiveEventLoop) {
            let windows = WindowSystem { event_loop };
            while let Ok(job) = self.jobs.try_recv() {
                job(&mut self.state, Some(&windows));
            }
        }
    }

    impl ApplicationHandler<()> for App {
        fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

        fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ()) {
            self.drain(event_loop);
        }

        fn window_event(
            &mut self,
            _event_loop: &ActiveEventLoop,
            window: WindowId,
            event: WindowEvent,
        ) {
            let state = &mut self.state;
            match event {
                WindowEvent::CloseRequested => {
                    state.editors.close_window(window, &mut state.instances)
                }
                WindowEvent::Resized(size) => state.editors.window_resized(
                    window,
                    GuiSize {
                        width: size.width,
                        height: size.height,
                    },
                    &mut state.instances,
                ),
                _ => {}
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            self.drain(event_loop);
            self.state.service(Some(&WindowSystem { event_loop }));
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + SERVICE_INTERVAL));
        }
    }

    fn build_event_loop() -> Option<EventLoop<()>> {
        let mut builder = EventLoop::<()>::with_user_event();
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            // Plugins embed in X11 windows, so prefer X11 (XWayland on Wayland desktops).
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11().with_any_thread(true);
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| builder.build()))
            .ok()?
            .ok()
    }

    /// Runs the main thread: the event loop when a display opens, the headless loop otherwise.
    pub(crate) fn run(jobs: Receiver<Job>, ready: SyncSender<Option<Waker>>) {
        let Some(event_loop) = build_event_loop() else {
            let _ = ready.send(None);
            crate::main_thread::headless_loop(jobs);
            return;
        };
        let _ = ready.send(Some(Waker(event_loop.create_proxy())));
        let mut app = App {
            state: MainState::default(),
            jobs,
        };
        if let Err(error) = event_loop.run_app(&mut app) {
            eprintln!("aura: CLAP main thread event loop stopped: {error}");
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use clack_extensions::gui::{GuiSize, Window as ClapWindow};

    pub(crate) type WindowKey = ();

    /// No window system on this platform yet; floating plugin GUIs only.
    pub(crate) struct WindowSystem<'a>(
        std::marker::PhantomData<&'a ()>,
        std::convert::Infallible,
    );

    impl WindowSystem<'_> {
        pub(crate) fn create_window(
            &self,
            _title: &str,
            _size: GuiSize,
            _resizable: bool,
        ) -> Result<HostWindow, String> {
            match self.1 {}
        }
    }

    pub(crate) enum HostWindow {}

    impl HostWindow {
        pub(crate) fn key(&self) -> WindowKey {
            match *self {}
        }
        pub(crate) fn clap_parent(&self) -> Option<ClapWindow<'static, 'static>> {
            match *self {}
        }
        pub(crate) fn set_size(&self, _size: GuiSize) {
            match *self {}
        }
        pub(crate) fn focus(&self) {
            match *self {}
        }
    }

    pub(crate) enum Waker {}

    impl Waker {
        pub(crate) fn wake(&self) {
            match *self {}
        }
    }

    pub(crate) fn run(jobs: Receiver<Job>, ready: SyncSender<Option<Waker>>) {
        let _ = ready.send(None);
        crate::main_thread::headless_loop(jobs);
    }
}

pub(crate) use imp::{run, HostWindow, WindowKey, WindowSystem, Waker};
