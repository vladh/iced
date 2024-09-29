#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(dead_code)]

//! Create interactive, native cross-platform applications for WGPU.
mod state;
mod window_manager;

pub use state::State;

use crate::conversion;
use crate::core;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::time::Instant;
use crate::core::widget::operation;
use crate::core::window;
use crate::core::{Color, Element, Point, Size, Theme};
use crate::futures::futures::channel::mpsc;
use crate::futures::futures::channel::oneshot;
use crate::futures::futures::task;
use crate::futures::futures::{Future, StreamExt};
use crate::futures::subscription::{self, Subscription};
use crate::futures::{Executor, Runtime};
use crate::graphics;
use crate::graphics::{compositor, Compositor};
use crate::runtime::user_interface::{self, UserInterface};
use crate::runtime::Debug;
use crate::runtime::{self, Action, Task};
use crate::{Clipboard, Error, Proxy, Settings};

use window_manager::WindowManager;

use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::mem::ManuallyDrop;
use std::sync::Arc;

/// An interactive, native, cross-platform, multi-windowed application.
///
/// This trait is the main entrypoint of multi-window Iced. Once implemented, you can run
/// your GUI application by simply calling [`run`]. It will run in
/// its own window.
///
/// A [`Program`] can execute asynchronous actions by returning a
/// [`Task`] in some of its methods.
///
/// When using a [`Program`] with the `debug` feature enabled, a debug view
/// can be toggled by pressing `F12`.
pub trait Program
where
    Self: Sized,
    Self::Theme: DefaultStyle,
{
    /// The type of __messages__ your [`Program`] will produce.
    type Message: std::fmt::Debug + Send;

    /// The theme used to draw the [`Program`].
    type Theme;

    /// The [`Executor`] that will run commands and subscriptions.
    ///
    /// The [default executor] can be a good starting point!
    ///
    /// [`Executor`]: Self::Executor
    /// [default executor]: crate::futures::backend::default::Executor
    type Executor: Executor;

    /// The graphics backend to use to draw the [`Program`].
    type Renderer: core::Renderer + core::text::Renderer;

    /// The data needed to initialize your [`Program`].
    type Flags;

    /// Initializes the [`Program`] with the flags provided to
    /// [`run`] as part of the [`Settings`].
    ///
    /// Here is where you should return the initial state of your app.
    ///
    /// Additionally, you can return a [`Task`] if you need to perform some
    /// async action in the background on startup. This is useful if you want to
    /// load state from a file, perform an initial HTTP request, etc.
    fn new(flags: Self::Flags) -> (Self, Task<Self::Message>);

    /// Returns the current title of the [`Program`].
    ///
    /// This title can be dynamic! The runtime will automatically update the
    /// title of your application when necessary.
    fn title(&self, window: window::Id) -> String;

    /// Handles a __message__ and updates the state of the [`Program`].
    ///
    /// This is where you define your __update logic__. All the __messages__,
    /// produced by either user interactions or commands, will be handled by
    /// this method.
    ///
    /// Any [`Task`] returned will be executed immediately in the background by the
    /// runtime.
    fn update(&mut self, message: Self::Message) -> Task<Self::Message>;

    /// Returns the widgets to display in the [`Program`] for the `window`.
    ///
    /// These widgets can produce __messages__ based on user interaction.
    fn view(
        &self,
        window: window::Id,
    ) -> Element<'_, Self::Message, Self::Theme, Self::Renderer>;

    /// Returns the current `Theme` of the [`Program`].
    fn theme(&self, window: window::Id) -> Self::Theme;

    /// Returns the `Style` variation of the `Theme`.
    fn style(&self, theme: &Self::Theme) -> Appearance {
        theme.default_style()
    }

    /// Returns the event `Subscription` for the current state of the
    /// application.
    ///
    /// The messages produced by the `Subscription` will be handled by
    /// [`update`](#tymethod.update).
    ///
    /// A `Subscription` will be kept alive as long as you keep returning it!
    ///
    /// By default, it returns an empty subscription.
    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::none()
    }

    /// Returns the scale factor of the window of the [`Program`].
    ///
    /// It can be used to dynamically control the size of the UI at runtime
    /// (i.e. zooming).
    ///
    /// For instance, a scale factor of `2.0` will make widgets twice as big,
    /// while a scale factor of `0.5` will shrink them to half their size.
    ///
    /// By default, it returns `1.0`.
    #[allow(unused_variables)]
    fn scale_factor(&self, window: window::Id) -> f64 {
        1.0
    }
}

/// The appearance of a program.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Appearance {
    /// The background [`Color`] of the application.
    pub background_color: Color,

    /// The default text [`Color`] of the application.
    pub text_color: Color,
}

/// The default style of a [`Program`].
pub trait DefaultStyle {
    /// Returns the default style of a [`Program`].
    fn default_style(&self) -> Appearance;
}

impl DefaultStyle for Theme {
    fn default_style(&self) -> Appearance {
        default(self)
    }
}

/// The default [`Appearance`] of a [`Program`] with the built-in [`Theme`].
pub fn default(theme: &Theme) -> Appearance {
    let palette = theme.extended_palette();

    Appearance {
        background_color: palette.background.base.color,
        text_color: palette.background.base.text,
    }
}
/// Runs a [`Program`] with an executor, compositor, and the provided
/// settings.
pub fn run<P, C>(
    settings: Settings,
    graphics_settings: graphics::Settings,
    window_settings: Option<window::Settings>,
    flags: P::Flags,
) -> Result<(), Error>
where
    P: Program + 'static,
    C: Compositor<Renderer = P::Renderer> + 'static,
    P::Theme: DefaultStyle,
{
    println!("iced/winit/src/program.rs#run()");
    use winit::event_loop::EventLoop;

    let mut debug = Debug::new();
    debug.startup_started();

    let event_loop = EventLoop::with_user_event()
        .build()
        .expect("Create event loop");

    let (proxy, worker) = Proxy::new(event_loop.create_proxy());

    let mut runtime = {
        let executor =
            P::Executor::new().map_err(Error::ExecutorCreationFailed)?;
        executor.spawn(worker);

        Runtime::new(executor, proxy.clone())
    };

    let (program, task) = runtime.enter(|| P::new(flags));
    let is_daemon = window_settings.is_none();

    let task = if let Some(window_settings) = window_settings {
        let mut task = Some(task);

        let (_id, open) = runtime::window::open(window_settings);

        open.then(move |_| task.take().unwrap_or(Task::none()))
    } else {
        task
    };

    if let Some(stream) = runtime::task::into_stream(task) {
        runtime.run(stream);
    }

    runtime.track(subscription::into_recipes(
        runtime.enter(|| program.subscription().map(Action::Output)),
    ));

    let (boot_sender, boot_receiver) = oneshot::channel();
    let (event_sender, event_receiver) = mpsc::unbounded();
    let (control_sender, control_receiver) = mpsc::unbounded();

    let instance = Box::pin(run_instance::<P, C>(
        program,
        runtime,
        proxy.clone(),
        debug,
        boot_receiver,
        event_receiver,
        control_sender,
        is_daemon,
    ));

    let context = task::Context::from_waker(task::noop_waker_ref());

    struct Runner<Message: 'static, F, C> {
        instance: std::pin::Pin<Box<F>>,
        context: task::Context<'static>,
        id: Option<String>,
        boot: Option<BootConfig<C>>,
        sender: mpsc::UnboundedSender<Event<Action<Message>>>,
        receiver: mpsc::UnboundedReceiver<Control>,
        error: Option<Error>,
    }

    struct BootConfig<C> {
        sender: oneshot::Sender<Boot<C>>,
        fonts: Vec<Cow<'static, [u8]>>,
        graphics_settings: graphics::Settings,
    }

    let runner = Runner {
        instance,
        context,
        id: settings.id,
        boot: Some(BootConfig {
            sender: boot_sender,
            fonts: settings.fonts,
            graphics_settings,
        }),
        sender: event_sender,
        receiver: control_receiver,
        error: None,
    };

    impl<Message, F, C> winit::application::ApplicationHandler<Action<Message>>
        for Runner<Message, F, C>
    where
        Message: std::fmt::Debug,
        F: Future<Output = ()>,
        C: Compositor + 'static,
    {
        fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            println!("iced/winit/src/program.rs#resumed()");
            let Some(BootConfig {
                sender,
                fonts,
                graphics_settings,
            }) = self.boot.take()
            else {
                return;
            };

            let window = {
                let attributes = winit::window::WindowAttributes::default();

                match event_loop.create_window(attributes.with_visible(false)) {
                    Ok(window) => Arc::new(window),
                    Err(error) => {
                        self.error = Some(Error::WindowCreationFailed(error));
                        event_loop.exit();
                        return;
                    }
                }
            };

            let finish_boot = async move {
                let mut compositor =
                    C::new(graphics_settings, window.clone()).await?;

                for font in fonts {
                    compositor.load_font(font);
                }

                sender
                    .send(Boot { compositor })
                    .ok()
                    .expect("Send boot event");

                Ok::<_, graphics::Error>(())
            };

            if let Err(error) =
                crate::futures::futures::executor::block_on(finish_boot)
            {
                self.error = Some(Error::GraphicsCreationFailed(error));
                event_loop.exit();
            }
        }

        fn new_events(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            cause: winit::event::StartCause,
        ) {
            println!("iced/winit/src/program.rs#new_events()");
            if self.boot.is_some() {
                return;
            }

            // self.process_event(
            //     event_loop,
            //     Event::EventLoopAwakened(winit::event::Event::NewEvents(cause)),
            // );
        }

        fn window_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            window_id: winit::window::WindowId,
            event: winit::event::WindowEvent,
        ) {
            println!("iced/winit/src/program.rs#window_event()");

            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::WindowEvent {
                    window_id,
                    event,
                }),
            );
        }

        fn user_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            action: Action<Message>,
        ) {
            println!("iced/winit/src/program.rs#user_event()");
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::UserEvent(
                    action,
                )),
            );
        }

        fn received_url(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            url: String,
        ) {
        }

        fn about_to_wait(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
        ) {
            println!("iced/winit/src/program.rs#about_to_wait()");
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::AboutToWait),
            );
        }
    }

    impl<Message, F, C> Runner<Message, F, C>
    where
        F: Future<Output = ()>,
        C: Compositor,
    {
        fn process_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            event: Event<Action<Message>>,
        ) {
            println!("iced/winit/src/program.rs#process_event()");
            if event_loop.exiting() {
                return;
            }

            self.sender.start_send(event).expect("Send event");

            loop {
                println!("iced/winit/src/program.rs#process_event(): loop");
                let poll = self.instance.as_mut().poll(&mut self.context);
                println!("iced/winit/src/program.rs#process_event(): polled");

                match poll {
                    task::Poll::Pending => match self.receiver.try_next() {
                        Ok(Some(control)) => match control {
                            // Control::ChangeFlow(flow) => {
                            //     println!("ChangeFlow({:?})", flow);
                            //     use winit::event_loop::ControlFlow;

                            //     match (event_loop.control_flow(), flow) {
                            //         (
                            //             ControlFlow::WaitUntil(current),
                            //             ControlFlow::WaitUntil(new),
                            //         ) if new < current => {}
                            //         (
                            //             ControlFlow::WaitUntil(target),
                            //             ControlFlow::Wait,
                            //         ) if target > Instant::now() => {}
                            //         _ => {
                            //             event_loop.set_control_flow(ControlFlow::Wait);
                            //         }
                            //     }
                            // }
                            Control::CreateWindow {
                                id,
                                settings,
                                title,
                                monitor,
                                on_open,
                            } => {
                                println!("iced/winit/src/program.rs#process_event(): Control::CreateWindow");
                                let exit_on_close_request =
                                    settings.exit_on_close_request;

                                let visible = settings.visible;

                                let window_attributes =
                                    conversion::window_attributes(
                                        settings,
                                        &title,
                                        monitor
                                            .or(event_loop.primary_monitor()),
                                        self.id.clone(),
                                    )
                                    .with_visible(false);

                                log::info!("Window attributes for id `{id:#?}`: {window_attributes:#?}");

                                let window = event_loop
                                    .create_window(window_attributes)
                                    .expect("Create window");

                                self.process_event(
                                    event_loop,
                                    Event::WindowCreated {
                                        id,
                                        window,
                                        exit_on_close_request,
                                        make_visible: visible,
                                        on_open,
                                    },
                                );
                            }
                            _ => {}
                            // Control::Exit => {
                            //     event_loop.exit();
                            // }
                        },
                        _ => {
                            break;
                        }
                    },
                    task::Poll::Ready(_) => {
                        println!("iced/winit/src/program.rs#process_event(): task::Poll::Ready");
                        event_loop.exit();
                        break;
                    }
                };
            }
        }
    }

    {
        let mut runner = runner;
        println!("iced/winit/src/program.rs#run(): calling run_app()");
        let _ = event_loop.run_app(&mut runner);
        println!("iced/winit/src/program.rs#run(): run_app() done");

        runner.error.map(Err).unwrap_or(Ok(()))
    }
}

struct Boot<C> {
    compositor: C,
}

#[derive(Debug)]
enum Event<Message: 'static> {
    WindowCreated {
        id: window::Id,
        window: winit::window::Window,
        exit_on_close_request: bool,
        make_visible: bool,
        on_open: oneshot::Sender<window::Id>,
    },
    EventLoopAwakened(winit::event::Event<Message>),
}

#[derive(Debug)]
enum Control {
    ChangeFlow(winit::event_loop::ControlFlow),
    Exit,
    CreateWindow {
        id: window::Id,
        settings: window::Settings,
        title: String,
        monitor: Option<winit::monitor::MonitorHandle>,
        on_open: oneshot::Sender<window::Id>,
    },
}

async fn run_instance<P, C>(
    mut program: P,
    mut runtime: Runtime<P::Executor, Proxy<P::Message>, Action<P::Message>>,
    mut proxy: Proxy<P::Message>,
    mut debug: Debug,
    boot: oneshot::Receiver<Boot<C>>,
    mut event_receiver: mpsc::UnboundedReceiver<Event<Action<P::Message>>>,
    mut control_sender: mpsc::UnboundedSender<Control>,
    is_daemon: bool,
) where
    P: Program + 'static,
    C: Compositor<Renderer = P::Renderer> + 'static,
    P::Theme: DefaultStyle,
{
    use winit::event;
    use winit::event_loop::ControlFlow;

    let Boot { mut compositor } = boot.await.expect("Receive boot");

    let mut window_manager = WindowManager::new();
    let mut is_window_opening = !is_daemon;

    let mut events = Vec::new();
    let mut messages = Vec::new();
    let mut actions = 0;

    let mut ui_caches = FxHashMap::default();
    let mut user_interfaces = ManuallyDrop::new(FxHashMap::default());
    let mut clipboard = Clipboard::unconnected();

    debug.startup_finished();

    loop {
        println!("iced/winit/src/program.rs#run_instance(): starting loop");
        // Empty the queue if possible
        let event = if let Ok(event) = event_receiver.try_next() {
            event
        } else {
            event_receiver.next().await
        };

        let Some(event) = event else {
            break;
        };

        println!("iced/winit/src/program.rs#run_instance(): (counter {}) (event {:?})", debug.counter, event);
        debug.counter += 1;
        match event {
            Event::WindowCreated {
                id,
                window,
                exit_on_close_request,
                make_visible,
                on_open,
            } => {
                let window = window_manager.insert(
                    id,
                    Arc::new(window),
                    &program,
                    &mut compositor,
                    exit_on_close_request,
                );

                // let logical_size = window.state.logical_size();

                // let _ = user_interfaces.insert(
                //     id,
                //     build_user_interface(
                //         &program,
                //         user_interface::Cache::default(),
                //         &mut window.renderer,
                //         logical_size,
                //         &mut debug,
                //         id,
                //     ),
                // );
                // let _ = ui_caches.insert(id, user_interface::Cache::default());

                // if make_visible {
                //     window.raw.set_visible(true);
                // }

                // events.push((
                //     id,
                //     core::Event::Window(window::Event::Opened {
                //         position: window.position(),
                //         size: window.size(),
                //     }),
                // ));

                // if clipboard.window_id().is_none() {
                //     clipboard = Clipboard::connect(window.raw.clone());
                // }

                let _ = on_open.send(id);
                is_window_opening = false;
            }
            Event::EventLoopAwakened(event) => {
                println!("iced/winit/src/program.rs#run_instance(): Event::EventLoopAwakened");
                match event {
                    event::Event::NewEvents(
                        event::StartCause::Init
                        | event::StartCause::ResumeTimeReached { .. },
                    ) => {
                        // for (_id, window) in window_manager.iter_mut() {
                        //     println!("requesting redraw because init or resume time reached");
                        //     window.raw.request_redraw();
                        // }
                    }
                    event::Event::UserEvent(action) => {
                        println!("iced/winit/src/program.rs#run_instance(): event::Event::UserEvent({:?})", action);
                        run_action(
                            action,
                            &program,
                            &mut compositor,
                            &mut events,
                            &mut messages,
                            &mut clipboard,
                            &mut control_sender,
                            &mut debug,
                            &mut user_interfaces,
                            &mut window_manager,
                            &mut ui_caches,
                            &mut is_window_opening,
                        );
                        actions += 1;
                    }
                    event::Event::WindowEvent {
                        window_id: id,
                        event: event::WindowEvent::RedrawRequested,
                        ..
                    } => {
                        let Some((id, window)) =
                            window_manager.get_mut_alias(id)
                        else {
                            continue;
                        };

                        // TODO: Avoid redrawing all the time by forcing widgets to
                        // request redraws on state changes
                        //
                        // Then, we can use the `interface_state` here to decide if a redraw
                        // is needed right away, or simply wait until a specific time.
                        // let redraw_event = core::Event::Window(
                        //     window::Event::RedrawRequested(Instant::now()),
                        // );

                        // let cursor = window.state.cursor();

                        // let ui = user_interfaces
                        //     .get_mut(&id)
                        //     .expect("Get user interface");

                        // let (ui_state, _) = ui.update(
                        //     &[redraw_event.clone()],
                        //     cursor,
                        //     &mut window.renderer,
                        //     &mut clipboard,
                        //     &mut messages,
                        // );

                        // debug.draw_started();
                        // let new_mouse_interaction = ui.draw(
                        //     &mut window.renderer,
                        //     window.state.theme(),
                        //     &renderer::Style {
                        //         text_color: window.state.text_color(),
                        //     },
                        //     cursor,
                        // );
                        // debug.draw_finished();

                        // if new_mouse_interaction != window.mouse_interaction {
                        //     window.raw.set_cursor(
                        //         conversion::mouse_interaction(
                        //             new_mouse_interaction,
                        //         ),
                        //     );

                        //     window.mouse_interaction = new_mouse_interaction;
                        // }

                        // runtime.broadcast(subscription::Event::Interaction {
                        //     window: id,
                        //     event: redraw_event,
                        //     status: core::event::Status::Ignored,
                        // });

                        // let _ = control_sender.start_send(Control::ChangeFlow(
                        //     match ui_state {
                        //         user_interface::State::Updated {
                        //             redraw_request: Some(redraw_request),
                        //         } => match redraw_request {
                        //             window::RedrawRequest::NextFrame => {
                        //                 println!("requesting redraw because next frame?");
                        //                 window.raw.request_redraw();

                        //                 ControlFlow::Wait
                        //             }
                        //             window::RedrawRequest::At(at) => {
                        //                 ControlFlow::WaitUntil(at)
                        //             }
                        //         },
                        //         _ => ControlFlow::Wait,
                        //     },
                        // ));

                        // let physical_size = window.state.physical_size();

                        // if physical_size.width == 0 || physical_size.height == 0
                        // {
                        //     continue;
                        // }

                        // if window.viewport_version
                        //     != window.state.viewport_version()
                        // {
                        //     let logical_size = window.state.logical_size();

                        //     debug.layout_started();
                        //     let ui = user_interfaces
                        //         .remove(&id)
                        //         .expect("Remove user interface");

                        //     let _ = user_interfaces.insert(
                        //         id,
                        //         ui.relayout(logical_size, &mut window.renderer),
                        //     );
                        //     debug.layout_finished();

                        //     debug.draw_started();
                        //     let new_mouse_interaction = user_interfaces
                        //         .get_mut(&id)
                        //         .expect("Get user interface")
                        //         .draw(
                        //             &mut window.renderer,
                        //             window.state.theme(),
                        //             &renderer::Style {
                        //                 text_color: window.state.text_color(),
                        //             },
                        //             window.state.cursor(),
                        //         );
                        //     debug.draw_finished();

                        //     compositor.configure_surface(
                        //         &mut window.surface,
                        //         physical_size.width,
                        //         physical_size.height,
                        //     );

                        //     window.viewport_version =
                        //         window.state.viewport_version();
                        // }

                        debug.render_started();
                        let pre_present = || {
                            println!("iced/winit/src/program.rs#run_instance(): pre_present() called");
                            window.raw.pre_present_notify();
                        };
                        println!("iced/winit/src/program.rs#run_instance(): calling compositor.present()");
                        match compositor.present(
                            &mut window.renderer,
                            &mut window.surface,
                            window.state.viewport(),
                            window.state.background_color(),
                            Some(pre_present),
                            &debug.overlay(),
                        ) {
                            Ok(()) => {
                                println!("iced/winit/src/program.rs#run_instance(): compositor.present() done");
                                debug.render_finished();
                            }
                            Err(error) => {}
                        }
                    }
                    event::Event::WindowEvent {
                        event: window_event,
                        window_id,
                    } => {
                        // if !is_daemon
                        //     && matches!(
                        //         window_event,
                        //         winit::event::WindowEvent::Destroyed
                        //     )
                        //     && !is_window_opening
                        //     && window_manager.is_empty()
                        // {
                        //     control_sender
                        //         .start_send(Control::Exit)
                        //         .expect("Send control action");

                        //     continue;
                        // }

                        // let Some((id, window)) =
                        //     window_manager.get_mut_alias(window_id)
                        // else {
                        //     continue;
                        // };

                        // if matches!(
                        //     window_event,
                        //     winit::event::WindowEvent::CloseRequested
                        // ) && window.exit_on_close_request
                        // {
                        //     run_action(
                        //         Action::Window(runtime::window::Action::Close(
                        //             id,
                        //         )),
                        //         &program,
                        //         &mut compositor,
                        //         &mut events,
                        //         &mut messages,
                        //         &mut clipboard,
                        //         &mut control_sender,
                        //         &mut debug,
                        //         &mut user_interfaces,
                        //         &mut window_manager,
                        //         &mut ui_caches,
                        //         &mut is_window_opening,
                        //     );
                        // } else {
                        //     window.state.update(
                        //         &window.raw,
                        //         &window_event,
                        //         &mut debug,
                        //     );

                        //     if let Some(event) = conversion::window_event(
                        //         window_event,
                        //         window.state.scale_factor(),
                        //         window.state.modifiers(),
                        //     ) {
                        //         events.push((id, event));
                        //     }
                        // }
                    }
                    // event::Event::AboutToWait => {
                        // if events.is_empty() && messages.is_empty() {
                        //     continue;
                        // }

                        // debug.event_processing_started();
                        // let mut uis_stale = false;

                        // for (id, window) in window_manager.iter_mut() {
                        //     let mut window_events = vec![];

                        //     events.retain(|(window_id, event)| {
                        //         if *window_id == id {
                        //             window_events.push(event.clone());
                        //             false
                        //         } else {
                        //             true
                        //         }
                        //     });

                        //     if window_events.is_empty() && messages.is_empty() {
                        //         continue;
                        //     }

                        //     let (ui_state, statuses) = user_interfaces
                        //         .get_mut(&id)
                        //         .expect("Get user interface")
                        //         .update(
                        //             &window_events,
                        //             window.state.cursor(),
                        //             &mut window.renderer,
                        //             &mut clipboard,
                        //             &mut messages,
                        //         );

                        //     println!("requesting redraw because of ui");
                        //     window.raw.request_redraw();

                        //     if !uis_stale {
                        //         uis_stale = matches!(
                        //             ui_state,
                        //             user_interface::State::Outdated
                        //         );
                        //     }

                        //     for (event, status) in window_events
                        //         .into_iter()
                        //         .zip(statuses.into_iter())
                        //     {
                        //         runtime.broadcast(
                        //             subscription::Event::Interaction {
                        //                 window: id,
                        //                 event,
                        //                 status,
                        //             },
                        //         );
                        //     }
                        // }

                        // for (id, event) in events.drain(..) {
                        //     runtime.broadcast(
                        //         subscription::Event::Interaction {
                        //             window: id,
                        //             event,
                        //             status: core::event::Status::Ignored,
                        //         },
                        //     );
                        // }

                        // debug.event_processing_finished();

                        // if !messages.is_empty() || uis_stale {
                        //     let cached_interfaces: FxHashMap<
                        //         window::Id,
                        //         user_interface::Cache,
                        //     > = ManuallyDrop::into_inner(user_interfaces)
                        //         .drain()
                        //         .map(|(id, ui)| (id, ui.into_cache()))
                        //         .collect();

                        //     update(
                        //         &mut program,
                        //         &mut runtime,
                        //         &mut debug,
                        //         &mut messages,
                        //     );

                        //     for (id, window) in window_manager.iter_mut() {
                        //         window.state.synchronize(
                        //             &program,
                        //             id,
                        //             &window.raw,
                        //         );

                        //         println!("requesting redraw because of messages");
                        //         window.raw.request_redraw();
                        //     }

                        //     user_interfaces =
                        //         ManuallyDrop::new(build_user_interfaces(
                        //             &program,
                        //             &mut debug,
                        //             &mut window_manager,
                        //             cached_interfaces,
                        //         ));

                        //     if actions > 0 {
                        //         proxy.free_slots(actions);
                        //         actions = 0;
                        //     }
                        // }
                    // }
                    _ => {}
                }
            }
        }
    }

    let _ = ManuallyDrop::into_inner(user_interfaces);
}

/// Builds a window's [`UserInterface`] for the [`Program`].
fn build_user_interface<'a, P: Program>(
    program: &'a P,
    cache: user_interface::Cache,
    renderer: &mut P::Renderer,
    size: Size,
    debug: &mut Debug,
    id: window::Id,
) -> UserInterface<'a, P::Message, P::Theme, P::Renderer>
where
    P::Theme: DefaultStyle,
{
    println!("iced/winit/src/program.rs#build_user_interface()");
    debug.view_started();
    let view = program.view(id);
    debug.view_finished();

    debug.layout_started();
    let user_interface = UserInterface::build(view, size, cache, renderer);
    debug.layout_finished();

    user_interface
}

fn update<P: Program, E: Executor>(
    program: &mut P,
    runtime: &mut Runtime<E, Proxy<P::Message>, Action<P::Message>>,
    debug: &mut Debug,
    messages: &mut Vec<P::Message>,
) where
    P::Theme: DefaultStyle,
{
}

fn run_action<P, C>(
    action: Action<P::Message>,
    program: &P,
    compositor: &mut C,
    events: &mut Vec<(window::Id, core::Event)>,
    messages: &mut Vec<P::Message>,
    clipboard: &mut Clipboard,
    control_sender: &mut mpsc::UnboundedSender<Control>,
    debug: &mut Debug,
    interfaces: &mut FxHashMap<
        window::Id,
        UserInterface<'_, P::Message, P::Theme, P::Renderer>,
    >,
    window_manager: &mut WindowManager<P, C>,
    ui_caches: &mut FxHashMap<window::Id, user_interface::Cache>,
    is_window_opening: &mut bool,
) where
    P: Program,
    C: Compositor<Renderer = P::Renderer> + 'static,
    P::Theme: DefaultStyle,
{
    println!("iced/winit/src/program.rs#run_action()");
    use crate::runtime::clipboard;
    use crate::runtime::system;
    use crate::runtime::window;

    match action {
        Action::Window(action) => match action {
            window::Action::Open(id, settings, channel) => {
                let monitor = window_manager.last_monitor();

                control_sender
                    .start_send(Control::CreateWindow {
                        id,
                        settings,
                        title: program.title(id),
                        monitor,
                        on_open: channel,
                    })
                    .expect("Send control action");

                *is_window_opening = true;
            }
            _ => {}
        },
        _ => {}
    }
}

/// Build the user interface for every window.
pub fn build_user_interfaces<'a, P: Program, C>(
    program: &'a P,
    debug: &mut Debug,
    window_manager: &mut WindowManager<P, C>,
    mut cached_user_interfaces: FxHashMap<window::Id, user_interface::Cache>,
) -> FxHashMap<window::Id, UserInterface<'a, P::Message, P::Theme, P::Renderer>>
where
    C: Compositor<Renderer = P::Renderer>,
    P::Theme: DefaultStyle,
{
    println!("iced/winit/src/program.rs#build_user_interfaces()");
    cached_user_interfaces
        .drain()
        .filter_map(|(id, cache)| {
            let window = window_manager.get_mut(id)?;

            Some((
                id,
                build_user_interface(
                    program,
                    cache,
                    &mut window.renderer,
                    window.state.logical_size(),
                    debug,
                    id,
                ),
            ))
        })
        .collect()
}

/// Returns true if the provided event should cause a [`Program`] to
/// exit.
pub fn user_force_quit(
    event: &winit::event::WindowEvent,
    _modifiers: winit::keyboard::ModifiersState,
) -> bool {
    true
}
