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

// Ivars to store our app state
#[derive(Debug)]
struct AppDelegateIvars {
    status_item: std::cell::OnceCell<Retained<NSStatusItem>>,
    menu: std::cell::OnceCell<Retained<NSMenu>>,
    current_input: std::cell::RefCell<String>,
}

// Create our app delegate class
define_class!(
    // SAFETY:
    // - The superclass NSObject does not have any subclassing requirements.
    // - `AppDelegate` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

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

            // Get the current input selection (default to USB)
            let current = self.ivars().current_input.borrow();

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
                if *current == "USB" {
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
                if *current == "WiFi" {
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
                if *current == "Bluetooth" {
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
                if *current == "Optical" {
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

            // Update the current input
            *self.ivars().current_input.borrow_mut() = title.to_string();

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
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm);
        let this = this.set_ivars(AppDelegateIvars {
            status_item: std::cell::OnceCell::new(),
            menu: std::cell::OnceCell::new(),
            current_input: std::cell::RefCell::new("USB".to_string()),
        });
        unsafe { msg_send![super(this), init] }
    }
}

fn main() {
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
