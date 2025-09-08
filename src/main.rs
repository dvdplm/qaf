#![allow(unsafe_op_in_unsafe_fn)]

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DeclaredClass, MainThreadOnly, Message, define_class};
use objc2::{MainThreadMarker, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use tracing::info;

// Channel for communication between UI and speaker controller
use tokio::sync::mpsc;

// Speaker control commands
#[derive(Debug, Clone)]
pub enum SpeakerCommand {
    SetInput(InputSource),
    GetStatus,
    PowerOn,
    PowerOff,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputSource {
    USB,
    WiFi,
    Bluetooth,
    Optical,
}

impl InputSource {
    fn as_str(&self) -> &'static str {
        match self {
            InputSource::USB => "USB",
            InputSource::WiFi => "WiFi",
            InputSource::Bluetooth => "Bluetooth",
            InputSource::Optical => "Optical",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "USB" => Some(InputSource::USB),
            "WiFi" => Some(InputSource::WiFi),
            "Bluetooth" => Some(InputSource::Bluetooth),
            "Optical" => Some(InputSource::Optical),
            _ => None,
        }
    }
}

mod speaker {
    use super::*;
    use tracing::info;

    pub struct SpeakerController {
        rx: mpsc::UnboundedReceiver<SpeakerCommand>,
        // Add your speaker API endpoint here
        // base_url: String,
    }

    impl SpeakerController {
        pub fn new(rx: mpsc::UnboundedReceiver<SpeakerCommand>) -> Self {
            Self {
                rx,
                // base_url: "http://speaker.local".to_string(),
            }
        }

        pub async fn run(mut self) {
            info!("Speaker controller started");

            while let Some(command) = self.rx.recv().await {
                match command {
                    SpeakerCommand::SetInput(input) => {
                        info!("Setting input to: {:?}", input);
                        // TODO: Make actual HTTP request
                        // Example:
                        // let client = reqwest::Client::new();
                        // let res = client.post(&format!("{}/input", self.base_url))
                        //     .json(&json!({ "source": input.as_str() }))
                        //     .send()
                        //     .await;
                    }
                    SpeakerCommand::GetStatus => {
                        info!("Getting speaker status");
                        // TODO: Make actual HTTP request
                    }
                    SpeakerCommand::PowerOn => {
                        info!("Powering on speakers");
                        // TODO: Make actual HTTP request
                    }
                    SpeakerCommand::PowerOff => {
                        info!("Powering off speakers");
                        // TODO: Make actual HTTP request
                    }
                }
            }

            info!("Speaker controller shutting down");
        }
    }
}

mod menubar {
    use super::*;
    use std::cell::{OnceCell, RefCell};
    use std::sync::{Arc, Mutex, OnceLock};

    // Store the sender globally so we can access it from the menu callbacks
    static SPEAKER_TX: OnceLock<Arc<Mutex<mpsc::UnboundedSender<SpeakerCommand>>>> =
        OnceLock::new();

    // Ivars to store our app state
    #[derive(Debug)]
    pub struct AppDelegateIvars {
        status_item: OnceCell<Retained<NSStatusItem>>,
        menu: OnceCell<Retained<NSMenu>>,
        current_input: RefCell<InputSource>,
    }

    // Create our app delegate class
    define_class!(
        // SAFETY:
        // - The superclass NSObject does not have any subclassing requirements.
        // - `AppDelegate` does not implement `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = AppDelegateIvars]
        pub struct AppDelegate;

        unsafe impl NSObjectProtocol for AppDelegate {}

        unsafe impl NSApplicationDelegate for AppDelegate {
            #[unsafe(method(applicationDidFinishLaunching:))]
            fn did_finish_launching(&self, _notification: &NSObject) {
                info!("Application did finish launching");

                let mtm = MainThreadMarker::from(self);

                // Get the system status bar
                let status_bar = unsafe { NSStatusBar::systemStatusBar() };

                // Create a status item with variable length (-1.0 for variable length)
                let status_item = unsafe { status_bar.statusItemWithLength(-1.0) };

                // Create the menu
                let menu = NSMenu::new(mtm);

                // Get the current input selection
                let current = *self.ivars().current_input.borrow();

                // Create menu items
                let usb_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str("USB"),
                        Some(objc2::sel!(menuItemClicked:)),
                        &NSString::from_str(""),
                    )
                };
                unsafe {
                    usb_item.setTarget(Some(&self.retain()));
                    usb_item.setTag(1);
                    if current == InputSource::USB {
                        let _: () = msg_send![&usb_item, setState: 1i64];
                    }
                }

                let wifi_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str("WiFi"),
                        Some(objc2::sel!(menuItemClicked:)),
                        &NSString::from_str(""),
                    )
                };
                unsafe {
                    wifi_item.setTarget(Some(&self.retain()));
                    wifi_item.setTag(2);
                    if current == InputSource::WiFi {
                        let _: () = msg_send![&wifi_item, setState: 1i64];
                    }
                }

                let bluetooth_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str("Bluetooth"),
                        Some(objc2::sel!(menuItemClicked:)),
                        &NSString::from_str(""),
                    )
                };
                unsafe {
                    bluetooth_item.setTarget(Some(&self.retain()));
                    bluetooth_item.setTag(3);
                    if current == InputSource::Bluetooth {
                        let _: () = msg_send![&bluetooth_item, setState: 1i64];
                    }
                }

                let optical_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str("Optical"),
                        Some(objc2::sel!(menuItemClicked:)),
                        &NSString::from_str(""),
                    )
                };
                unsafe {
                    optical_item.setTarget(Some(&self.retain()));
                    optical_item.setTag(4);
                    if current == InputSource::Optical {
                        let _: () = msg_send![&optical_item, setState: 1i64];
                    }
                }

                // Add items to menu
                menu.addItem(&usb_item);
                menu.addItem(&wifi_item);
                menu.addItem(&bluetooth_item);
                menu.addItem(&optical_item);

                // Add separator
                let separator = NSMenuItem::separatorItem(mtm);
                menu.addItem(&separator);

                // Add quit item
                let quit_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str("Quit"),
                        Some(objc2::sel!(quitClicked:)),
                        &NSString::from_str(""),
                    )
                };
                unsafe { quit_item.setTarget(Some(&self.retain())) };
                menu.addItem(&quit_item);

                // Set the title text for now (we'll use an icon later)
                let title = NSString::from_str("qaf");
                unsafe {
                    let button = status_item
                        .button(mtm)
                        .expect("Status item should have a button");
                    button.setTitle(&title);
                }

                // Set the menu on the status item - it will show automatically on click
                unsafe {
                    status_item.setMenu(Some(&menu));
                }

                // Store the status item and menu in our ivars so they don't get deallocated
                self.ivars().status_item.set(status_item).ok();
                self.ivars().menu.set(menu).ok();

                info!("Status bar item created");
            }

        }
        impl AppDelegate {
            #[unsafe(method(menuItemClicked:))]
            fn menu_item_clicked(&self, sender: &NSMenuItem) {
                let title = unsafe { sender.title() };
                info!("Menu item clicked: {}", title);

                // Parse the input source
                if let Some(input) = InputSource::from_str(&title.to_string()) {
                    // Update the current input
                    *self.ivars().current_input.borrow_mut() = input;

                    // Update the menu item states
                    if let Some(menu) = self.ivars().menu.get() {
                        let item_count = unsafe { menu.numberOfItems() };
                        for i in 0..item_count {
                            if let Some(item) = unsafe { menu.itemAtIndex(i) } {
                                unsafe {
                                    if item.title().to_string() == title.to_string() {
                                        let _: () = msg_send![&item, setState: 1i64];
                                    } else {
                                        let _: () = msg_send![&item, setState: 0i64];
                                    }
                                }
                            }
                        }
                    }

                    // Send command to speaker controller
                    if let Some(tx) = SPEAKER_TX.get() {
                        if let Ok(tx) = tx.lock() {
                            let _ = tx.send(SpeakerCommand::SetInput(input));
                        }
                    }
                }
            }

            #[unsafe(method(quitClicked:))]
            fn quit_clicked(&self, _sender: &NSMenuItem) {
                info!("Quit clicked - exiting application");
                let app = NSApplication::sharedApplication(MainThreadMarker::from(self));
                unsafe {
                    app.terminate(None);
                }
            }
        }
    );

    impl AppDelegate {
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm);
            let this = this.set_ivars(AppDelegateIvars {
                status_item: OnceCell::new(),
                menu: OnceCell::new(),
                current_input: RefCell::new(InputSource::USB),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    pub fn run(tx: mpsc::UnboundedSender<SpeakerCommand>) {
        // Store the sender for use in menu callbacks
        let _ = SPEAKER_TX.set(Arc::new(Mutex::new(tx)));

        // Initialize tracing
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(tracing::Level::INFO.into()),
            )
            .init();

        info!("Starting qaf menubar app");

        // This is required for GUI apps on macOS
        let mtm = MainThreadMarker::new().expect("Must be run on the main thread");

        // Get the shared application instance
        let app = NSApplication::sharedApplication(mtm);

        // Set the activation policy to accessory (menubar app)
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        // Create and set our app delegate
        let delegate = AppDelegate::new(mtm);

        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

        info!("Running application");

        // Run the app
        app.run();
    }
}

fn main() {
    // Create channel for communication
    let (tx, rx) = mpsc::unbounded_channel::<SpeakerCommand>();

    // Spawn the async runtime in a separate thread
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        runtime.block_on(async {
            let controller = speaker::SpeakerController::new(rx);
            controller.run().await;
        });
    });

    // Run the UI on the main thread
    menubar::run(tx);
}
