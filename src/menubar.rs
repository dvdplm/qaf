use std::cell::{OnceCell, RefCell};
use std::sync::{Arc, Mutex, OnceLock};

use crate::{InputSource, SpeakerCommand, SpeakerStatus};

use objc2::{
    DeclaredClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSTimeInterval, NSTimer};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info};

// Store the sender globally so we can access it from the menu callbacks
static SPEAKER_TX: OnceLock<Arc<Mutex<mpsc::UnboundedSender<SpeakerCommand>>>> = OnceLock::new();

// Ivars to store our app state
#[derive(Debug)]
pub struct AppDelegateIvars {
    status_item: OnceCell<Retained<NSStatusItem>>,
    menu: OnceCell<Retained<NSMenu>>,
    current_input: RefCell<Option<InputSource>>,
    power_item: OnceCell<Retained<NSMenuItem>>,
    speaker_powered: RefCell<bool>,
    poll_rx: RefCell<UnboundedReceiver<SpeakerStatus>>,
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

            // Query speaker status first
            let current_input = if let Some(tx) = SPEAKER_TX.get() {
                if let Ok(tx) = tx.lock() {
                    let (status_tx, status_rx) = oneshot::channel();
                    let _ = tx.send(SpeakerCommand::GetStatus(status_tx));

                    // Wait for response (with timeout)
                    match std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async {
                            tokio::time::timeout(
                                tokio::time::Duration::from_secs(2),
                                status_rx
                            ).await
                        })
                    }).join() {
                        Ok(Ok(Ok(status))) => {
                            info!("Speaker status on startup: {:?}", status);
                            *self.ivars().speaker_powered.borrow_mut() = status.power == "powerOn";
                            status.source
                        }
                        _ => {
                            info!("Failed to get speaker status, defaulting to no selection");
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Update the stored current input
            *self.ivars().current_input.borrow_mut() = current_input;

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
                if current_input == Some(InputSource::USB) {
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
                if current_input == Some(InputSource::WiFi) {
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
                if current_input == Some(InputSource::Bluetooth) {
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
                if current_input == Some(InputSource::Optical) {
                    let _: () = msg_send![&optical_item, setState: 1i64];
                }
            }

            let tv_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str("Tv"),
                    Some(objc2::sel!(menuItemClicked:)),
                    &NSString::from_str(""),
                )
            };
            unsafe {
                tv_item.setTarget(Some(&self.retain()));
                tv_item.setTag(5);
                if current_input == Some(InputSource::Tv) {
                    let _: () = msg_send![&tv_item, setState: 1i64];
                }
            }

            // Add items to menu
            menu.addItem(&usb_item);
            menu.addItem(&wifi_item);
            menu.addItem(&bluetooth_item);
            menu.addItem(&optical_item);
            menu.addItem(&tv_item);

            // Add separator before power control
            let separator1 = NSMenuItem::separatorItem(mtm);
            menu.addItem(&separator1);

            // Add power on/off item
            let power_text = if *self.ivars().speaker_powered.borrow() {
                "Power Off"
            } else {
                "Power On"
            };
            let power_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(power_text),
                    Some(objc2::sel!(powerClicked:)),
                    &NSString::from_str(""),
                )
            };
            unsafe { power_item.setTarget(Some(&self.retain())) };
            menu.addItem(&power_item);
            self.ivars().power_item.set(power_item).ok();

            // Add separator before quit
            let separator2 = NSMenuItem::separatorItem(mtm);
            menu.addItem(&separator2);

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

            // Start timer to process poll updates
            let _timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    0.5 as NSTimeInterval, // Check every half second for faster discovery feedback
                    &self.retain(),
                    sel!(processPollUpdates:),
                    None,
                    true,
                )
            };
        }

    }
    impl AppDelegate {
        #[unsafe(method(processPollUpdates:))]
        fn process_poll_updates(&self, _timer: &NSTimer) {
            while let Ok(status) = self.ivars().poll_rx.borrow_mut().try_recv() {
                debug!("Processing poll update: {:?}", status);

                let is_powered = status.power == "powerOn";
                *self.ivars().speaker_powered.borrow_mut() = is_powered;
                *self.ivars().current_input.borrow_mut() = status.source;

                // Update power menu item text
                if let Some(power_item) = self.ivars().power_item.get() {
                    let text = if is_powered { "Power Off" } else { "Power On" };
                    unsafe {
                        power_item.setTitle(&NSString::from_str(text));
                    }
                }

                // Update menu checkmarks
                if let Some(menu) = self.ivars().menu.get() {
                    let item_count = unsafe { menu.numberOfItems() };
                    for i in 0..item_count {
                        if let Some(item) = unsafe { menu.itemAtIndex(i) } {
                            let title = unsafe { item.title() };
                            if let Some(input) = InputSource::from_ns_string(&title) {
                                unsafe {
                                    if status.source == Some(input) {
                                        let _: () = msg_send![&item, setState: 1i64];
                                    } else {
                                        let _: () = msg_send![&item, setState: 0i64];
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        #[unsafe(method(menuItemClicked:))]
        fn menu_item_clicked(&self, sender: &NSMenuItem) {
            let title = unsafe { sender.title() };
            debug!("Menu item clicked: {}", title);

            // Parse the input source
            if let Some(input) = InputSource::from_ns_string(&title) {
                // Update the current input
                *self.ivars().current_input.borrow_mut() = Some(input);

                // Update the menu item states
                if let Some(menu) = self.ivars().menu.get() {
                    let item_count = unsafe { menu.numberOfItems() };
                    for i in 0..item_count {
                        if let Some(item) = unsafe { menu.itemAtIndex(i) } {
                            unsafe {
                                if item.title() == title {
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

        #[unsafe(method(powerClicked:))]
        fn power_clicked(&self, _sender: &NSMenuItem) {
            let is_powered = *self.ivars().speaker_powered.borrow();
            info!("Power clicked - current state: {}", if is_powered { "on" } else { "off" });

            // Send appropriate command
            if let Some(tx) = SPEAKER_TX.get() {
                if let Ok(tx) = tx.lock() {
                    if is_powered {
                        let _ = tx.send(SpeakerCommand::PowerOff);
                        *self.ivars().speaker_powered.borrow_mut() = false;
                        *self.ivars().current_input.borrow_mut() = None;
                    } else {
                        let _ = tx.send(SpeakerCommand::PowerOn);
                        *self.ivars().speaker_powered.borrow_mut() = true;
                    }

                    // Update power menu item text
                    if let Some(power_item) = self.ivars().power_item.get() {
                        let new_text = if is_powered {
                            "Power Off"
                        } else {
                            "Power On"
                        };
                        unsafe {
                            power_item.setTitle(&NSString::from_str(new_text));
                        }
                    }

                    // Clear selection if powering off
                    if is_powered {
                        // Clear all checkmarks
                        if let Some(menu) = self.ivars().menu.get() {
                            let item_count = unsafe { menu.numberOfItems() };
                            for i in 0..item_count {
                                if let Some(item) = unsafe { menu.itemAtIndex(i) } {
                                    unsafe {
                                        let _: () = msg_send![&item, setState: 0i64];
                                    }
                                }
                            }
                        }
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
    pub fn new(
        mtm: MainThreadMarker,
        poll_rx: mpsc::UnboundedReceiver<SpeakerStatus>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm);
        let this = this.set_ivars(AppDelegateIvars {
            status_item: OnceCell::new(),
            menu: OnceCell::new(),
            current_input: RefCell::new(None),
            power_item: OnceCell::new(),
            speaker_powered: RefCell::new(false),
            poll_rx: RefCell::new(poll_rx),
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn run(
    tx: mpsc::UnboundedSender<SpeakerCommand>,
    poll_rx: mpsc::UnboundedReceiver<SpeakerStatus>,
) {
    // Store the sender for use in menu callbacks - do this BEFORE creating the app delegate
    let _ = SPEAKER_TX.set(Arc::new(Mutex::new(tx)));

    // This is required for GUI apps on macOS
    let mtm = MainThreadMarker::new().expect("Must be run on the main thread");

    // Get the shared application instance
    let app = NSApplication::sharedApplication(mtm);

    // Set the activation policy to accessory (menubar app)
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    // Create and set our app delegate
    let delegate = AppDelegate::new(mtm, poll_rx);

    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    info!("Running application");

    // Run the app
    app.run();
}
