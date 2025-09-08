#![allow(unsafe_op_in_unsafe_fn)]

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DeclaredClass, MainThreadOnly, Message, define_class};
use objc2::{MainThreadMarker, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use tracing::info;

// Ivars to store our app state
#[derive(Debug)]
struct AppDelegateIvars {
    status_item: std::cell::OnceCell<Retained<NSStatusItem>>,
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

            // Get the system status bar
            let status_bar = unsafe { NSStatusBar::systemStatusBar() };

            // Create a status item with variable length (-1.0 for variable length)
            let status_item = unsafe { status_bar.statusItemWithLength(-1.0) };

            let mtm = MainThreadMarker::from(self);
            // Set the title text for now (we'll use an icon later)
            let title = NSString::from_str("qaf");
            unsafe {
                let button = status_item
                    .button(mtm)
                    .expect("Status item should have a button");
                button.setTitle(&title);

                // Set the button action
                button.setTarget(Some(&self.retain()));
                button.setAction(Some(objc2::sel!(statusItemClicked:)));
            }

            // Store the status item in our ivars so it doesn't get deallocated
            self.ivars().status_item.set(status_item).ok();

            info!("Status bar item created");
        }

    }
    impl AppDelegate {
        #[unsafe(method(statusItemClicked:))]
        fn status_item_clicked(&self, _sender: &NSObject) {
            info!("clicked");
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm);
        let this = this.set_ivars(AppDelegateIvars {
            status_item: std::cell::OnceCell::new(),
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
