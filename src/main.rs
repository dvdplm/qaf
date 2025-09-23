#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DeclaredClass, MainThreadOnly, Message, define_class};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString, NSTimeInterval, NSTimer};
use tracing::{debug, error, info};

// Channel for communication between UI and speaker controller
use tokio::sync::mpsc;
use tokio::sync::oneshot;

// Speaker discovery and control commands
#[derive(Debug)]
pub enum SpeakerCommand {
    SetInput(InputSource),
    GetStatus(oneshot::Sender<SpeakerStatus>),
    PowerOn,
    PowerOff,
    PollUpdate(SpeakerStatus),
    SpeakerDiscovered(SpeakerInfo),
}

#[derive(Debug, Clone)]
pub struct SpeakerInfo {
    pub address: String,
    pub port: u16,
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct SpeakerStatus {
    pub power: String, // "standby" or "powerOn"
    pub source: Option<InputSource>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputSource {
    USB,
    WiFi,
    Bluetooth,
    Optical,
}

impl InputSource {
    fn to_kef_source(&self) -> &'static str {
        match self {
            InputSource::USB => "usb",
            InputSource::WiFi => "wifi",
            InputSource::Bluetooth => "bluetooth",
            InputSource::Optical => "tv", // KEF uses "tv" for optical input
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

    fn from_kef_source(s: &str) -> Option<Self> {
        match s {
            "usb" => Some(InputSource::USB),
            "wifi" => Some(InputSource::WiFi),
            "bluetooth" => Some(InputSource::Bluetooth),
            "tv" => Some(InputSource::Optical),
            _ => None,
        }
    }
}

mod speaker {
    use super::*;
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    use serde_json::json;

    pub struct SpeakerController {
        rx: mpsc::UnboundedReceiver<SpeakerCommand>,
        base_url: Option<String>,
        client: reqwest::Client,
    }

    impl SpeakerController {
        pub fn new(rx: mpsc::UnboundedReceiver<SpeakerCommand>) -> Self {
            Self {
                rx,
                base_url: None,
                client: reqwest::Client::new(),
            }
        }

        pub async fn discover_speakers(tx: mpsc::UnboundedSender<SpeakerCommand>) {
            tokio::task::spawn_blocking(move || {
                info!("Starting mDNS discovery for KEF speakers...");

                let service_type = "_kef-info._tcp.local.";
                let mdns = match ServiceDaemon::new() {
                    Ok(daemon) => daemon,
                    Err(e) => {
                        error!("Failed to create mDNS daemon: {}", e);
                        return;
                    }
                };

                let receiver = match mdns.browse(service_type) {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Failed to browse for KEF speakers: {}", e);
                        return;
                    }
                };

                info!("Searching for KEF speakers on the network...");

                while let Ok(event) = receiver.recv() {
                    if let ServiceEvent::ServiceResolved(info) = event {
                        info!("Found KEF speaker: {}", info.get_fullname());

                        // Get the first IPv4 address
                        if let Some(addr) = info.get_addresses().iter().find(|a| a.is_ipv4()) {
                            let port = info.get_port();
                            let name = info
                                .get_property("name")
                                .map(|p| p.val_str().to_string())
                                .unwrap_or_else(|| "Unknown KEF Speaker".to_string());
                            let model = info
                                .get_property("modelName")
                                .map(|p| p.val_str().to_string())
                                .unwrap_or_else(|| "Unknown Model".to_string());

                            info!(
                                "KEF Speaker discovered - Name: {}, Model: {}, Address: {}:{}",
                                name, model, addr, port
                            );

                            let speaker_info = SpeakerInfo {
                                address: addr.to_string(),
                                port,
                                name,
                                model,
                            };

                            let _ = tx.send(SpeakerCommand::SpeakerDiscovered(speaker_info));

                            // Stop mDNS discovery by calling shutdown
                            info!("Stopping mDNS discovery after finding first speaker");
                            drop(receiver);

                            match mdns.shutdown() {
                                Ok(shutdown_rx) => {
                                    // Wait for shutdown confirmation
                                    if let Ok(_) = shutdown_rx.recv() {
                                        info!("mDNS daemon shutdown successfully");
                                    } else {
                                        error!("Failed to receive mDNS shutdown confirmation");
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to shutdown mDNS daemon: {}", e);
                                }
                            }
                            return;
                        }
                    }
                }

                info!("mDNS discovery thread exiting (no more events)");
            });
        }

        pub async fn run(mut self) {
            info!("Speaker controller started, waiting for speaker discovery...");

            while let Some(command) = self.rx.recv().await {
                match command {
                    SpeakerCommand::SpeakerDiscovered(info) => {
                        info!(
                            "Speaker discovered, updating base URL to http://{}:{}",
                            info.address, info.port
                        );
                        self.base_url = Some(format!("http://{}:{}", info.address, info.port));
                    }
                    SpeakerCommand::SetInput(input) => {
                        if self.base_url.is_none() {
                            error!("Cannot set input: No speaker discovered yet");
                            continue;
                        }
                        info!("Setting input to: {:?}", input);

                        // First check if we need to power on
                        if let Ok(status) = self.get_speaker_status().await {
                            if status.power == "standby" {
                                info!("Speaker is in standby, powering on first");
                                if let Err(e) = self.power_on().await {
                                    error!("Failed to power on: {}", e);
                                    continue;
                                }
                                // Wait a bit for the speaker to power on
                                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                            }
                        }

                        if let Err(e) = self.set_input(input).await {
                            error!("Failed to set input: {}", e);
                        }
                    }
                    SpeakerCommand::GetStatus(tx) => {
                        if self.base_url.is_none() {
                            info!("Cannot get status: No speaker discovered yet");
                            let _ = tx.send(SpeakerStatus {
                                power: "unknown".to_string(),
                                source: None,
                            });
                            continue;
                        }
                        info!("Getting speaker status");
                        match self.get_speaker_status().await {
                            Ok(status) => {
                                let _ = tx.send(status);
                            }
                            Err(e) => {
                                error!("Failed to get status: {}", e);
                                let _ = tx.send(SpeakerStatus {
                                    power: "unknown".to_string(),
                                    source: None,
                                });
                            }
                        }
                    }
                    SpeakerCommand::PowerOn => {
                        if self.base_url.is_none() {
                            error!("Cannot power on: No speaker discovered yet");
                            continue;
                        }
                        info!("Powering on speakers");
                        if let Err(e) = self.power_on().await {
                            error!("Failed to power on: {}", e);
                        }
                    }
                    SpeakerCommand::PowerOff => {
                        if self.base_url.is_none() {
                            error!("Cannot power off: No speaker discovered yet");
                            continue;
                        }
                        info!("Powering off speakers");
                        if let Err(e) = self.power_off().await {
                            error!("Failed to power off: {}", e);
                        }
                    }
                    SpeakerCommand::PollUpdate(status) => {
                        // This is handled by the UI, just log it
                        debug!("Poll update received: {:?}", status);
                    }
                }
            }

            info!("Speaker controller shutting down");
        }

        async fn set_input(&self, input: InputSource) -> Result<(), Box<dyn std::error::Error>> {
            let base_url = self.base_url.as_ref().ok_or("No speaker URL")?;
            let source = input.to_kef_source();
            let value = json!({
                "type": "kefPhysicalSource",
                "kefPhysicalSource": source
            });

            let params = [
                ("path", "settings:/kef/play/physicalSource"),
                ("roles", "value"),
                ("value", &value.to_string()),
            ];

            let response = self
                .client
                .get(&format!("{}/api/setData", base_url))
                .query(&params)
                .send()
                .await?;

            let json: serde_json::Value = response.json().await?;
            debug!(
                "Set input response: {}",
                serde_json::to_string_pretty(&json)?
            );
            info!("Successfully set input to {:?}", input);

            Ok(())
        }

        async fn power_on(&self) -> Result<(), Box<dyn std::error::Error>> {
            let base_url = self.base_url.as_ref().ok_or("No speaker URL")?;
            let value = json!({
                "type": "kefPhysicalSource",
                "kefPhysicalSource": "powerOn"
            });

            let params = [
                ("path", "settings:/kef/play/physicalSource"),
                ("roles", "value"),
                ("value", &value.to_string()),
            ];

            let response = self
                .client
                .get(&format!("{}/api/setData", base_url))
                .query(&params)
                .send()
                .await?;

            let json: serde_json::Value = response.json().await?;
            debug!(
                "Power on response: {}",
                serde_json::to_string_pretty(&json)?
            );
            info!("Successfully powered on speakers");

            Ok(())
        }

        async fn power_off(&self) -> Result<(), Box<dyn std::error::Error>> {
            let base_url = self.base_url.as_ref().ok_or("No speaker URL")?;
            let value = json!({
                "type": "kefPhysicalSource",
                "kefPhysicalSource": "standby"
            });

            let params = [
                ("path", "settings:/kef/play/physicalSource"),
                ("roles", "value"),
                ("value", &value.to_string()),
            ];

            let response = self
                .client
                .get(&format!("{}/api/setData", base_url))
                .query(&params)
                .send()
                .await?;

            let json: serde_json::Value = response.json().await?;
            debug!(
                "Power off response: {}",
                serde_json::to_string_pretty(&json)?
            );
            info!("Successfully powered off speakers");

            Ok(())
        }

        async fn get_speaker_status(&self) -> Result<SpeakerStatus, Box<dyn std::error::Error>> {
            let base_url = self.base_url.as_ref().ok_or("No speaker URL")?;
            // Get power status
            let params = [
                ("path", "settings:/kef/host/speakerStatus"),
                ("roles", "value"),
            ];

            let response = self
                .client
                .get(&format!("{}/api/getData", base_url))
                .query(&params)
                .send()
                .await?;

            let power_json: serde_json::Value = response.json().await?;
            debug!(
                "Speaker power status response: {}",
                serde_json::to_string_pretty(&power_json)?
            );

            let power = power_json[0]["kefSpeakerStatus"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();

            // Get current source if powered on
            let source = if power == "powerOn" {
                let params = [
                    ("path", "settings:/kef/play/physicalSource"),
                    ("roles", "value"),
                ];

                let response = self
                    .client
                    .get(&format!("{}/api/getData", base_url))
                    .query(&params)
                    .send()
                    .await?;

                let source_json: serde_json::Value = response.json().await?;
                debug!(
                    "Speaker source response: {}",
                    serde_json::to_string_pretty(&source_json)?
                );

                let kef_source = source_json[0]["kefPhysicalSource"].as_str().unwrap_or("");

                InputSource::from_kef_source(kef_source)
            } else {
                None
            };

            Ok(SpeakerStatus { power, source })
        }
    }
}

mod menubar {
    use super::*;

    use std::cell::{OnceCell, RefCell};
    use std::sync::{Arc, Mutex, OnceLock};

    use tracing::trace;

    // Store the sender globally so we can access it from the menu callbacks
    static SPEAKER_TX: OnceLock<Arc<Mutex<mpsc::UnboundedSender<SpeakerCommand>>>> =
        OnceLock::new();

    // Store a channel for receiving poll updates
    static POLL_RX: OnceLock<Arc<Mutex<mpsc::UnboundedReceiver<SpeakerStatus>>>> = OnceLock::new();

    // Store speaker info once discovered
    static SPEAKER_INFO: OnceLock<Arc<Mutex<Option<SpeakerInfo>>>> = OnceLock::new();

    // Ivars to store our app state
    #[derive(Debug)]
    pub struct AppDelegateIvars {
        status_item: OnceCell<Retained<NSStatusItem>>,
        menu: OnceCell<Retained<NSMenu>>,
        current_input: RefCell<Option<InputSource>>,
        power_item: OnceCell<Retained<NSMenuItem>>,
        speaker_powered: RefCell<bool>,
        speaker_discovered: RefCell<bool>,
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

                // Add items to menu
                menu.addItem(&usb_item);
                menu.addItem(&wifi_item);
                menu.addItem(&bluetooth_item);
                menu.addItem(&optical_item);

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

                if !*self.ivars().speaker_discovered.borrow() {
                    if let Some(speaker_info) = SPEAKER_INFO.get() {
                        if let Ok(info_lock) = speaker_info.try_lock() {
                            if let Some(ref info) = *info_lock {
                                *self.ivars().speaker_discovered.borrow_mut() = true;
                                debug!("UI: Speaker discovered, enabling controls");
                                trace!("UI: Full speaker info: {info:?}");
                            }
                        }
                    }
                }

                if let Some(poll_rx) = POLL_RX.get() {
                    if let Ok(mut poll_rx) = poll_rx.try_lock() {
                        // Process all pending updates
                        while let Ok(status) = poll_rx.try_recv() {
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
                                        let title = unsafe { item.title().to_string() };
                                        if let Some(input) = InputSource::from_str(&title) {
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
                }
            }

            #[unsafe(method(menuItemClicked:))]
            fn menu_item_clicked(&self, sender: &NSMenuItem) {
                if !*self.ivars().speaker_discovered.borrow() {
                    info!("Ignoring menu click - no speaker discovered yet");
                    return;
                }

                let title = unsafe { sender.title() };
                info!("Menu item clicked: {}", title);

                // Parse the input source
                if let Some(input) = InputSource::from_str(&title.to_string()) {
                    // Update the current input
                    *self.ivars().current_input.borrow_mut() = Some(input);

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

            #[unsafe(method(powerClicked:))]
            fn power_clicked(&self, _sender: &NSMenuItem) {
                if !*self.ivars().speaker_discovered.borrow() {
                    info!("Ignoring power click - no speaker discovered yet");
                    return;
                }

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
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm);
            let this = this.set_ivars(AppDelegateIvars {
                status_item: OnceCell::new(),
                menu: OnceCell::new(),
                current_input: RefCell::new(None),
                power_item: OnceCell::new(),
                speaker_powered: RefCell::new(false),
                speaker_discovered: RefCell::new(false),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    pub fn run(
        tx: mpsc::UnboundedSender<SpeakerCommand>,
        poll_rx: mpsc::UnboundedReceiver<SpeakerStatus>,
        mut speaker_info_rx: mpsc::UnboundedReceiver<SpeakerInfo>,
    ) {
        // Store the sender for use in menu callbacks - do this BEFORE creating the app delegate
        let _ = SPEAKER_TX.set(Arc::new(Mutex::new(tx)));
        let _ = POLL_RX.set(Arc::new(Mutex::new(poll_rx)));
        let _ = SPEAKER_INFO.set(Arc::new(Mutex::new(None)));

        // Start processing speaker info in background
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                while let Some(info) = speaker_info_rx.recv().await {
                    if let Some(speaker_info) = SPEAKER_INFO.get() {
                        if let Ok(mut lock) = speaker_info.lock() {
                            info!("Storing discovered speaker info: {:?}", info);
                            *lock = Some(info);
                        }
                    }
                }
            });
        });

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
    // Initialize tracing first
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env(), // .add_directive(tracing::Level::DEBUG.into()),
        )
        .init();

    info!("Starting qaf menubar app");

    // Create channels for communication
    let (tx, mut rx) = mpsc::unbounded_channel::<SpeakerCommand>();
    let (poll_tx, poll_rx) = mpsc::unbounded_channel::<SpeakerStatus>();
    let (speaker_info_tx, speaker_info_rx) = mpsc::unbounded_channel::<SpeakerInfo>();

    let tx_discovery = tx.clone();
    let poll_tx_clone = poll_tx.clone();

    // Spawn the async runtime in a separate thread
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        runtime.block_on(async {
            // Shared state for discovered speaker
            let speaker_info = Arc::new(tokio::sync::RwLock::new(None::<SpeakerInfo>));

            // Start speaker discovery
            tokio::spawn(async move {
                speaker::SpeakerController::discover_speakers(tx_discovery.clone()).await;
            });

            // Process discovery events and update shared state
            let discovered_speaker_discovery = speaker_info.clone();
            let (internal_tx, internal_rx) = mpsc::unbounded_channel::<SpeakerCommand>();
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    match &cmd {
                        SpeakerCommand::SpeakerDiscovered(info) => {
                            info!("Speaker discovered: {}", info.name);
                            *discovered_speaker_discovery.write().await = Some(info.clone());
                            let _ = speaker_info_tx.send(info.clone());
                            let _ = internal_tx.send(cmd);
                        }
                        _ => {
                            let _ = internal_tx.send(cmd);
                        }
                    }
                }
            });

            // Start periodic polling task
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));

                loop {
                    // Check if we have discovered a speaker
                    let speaker = speaker_info.read().await.clone();
                    if let Some(speaker_info) = speaker {
                        let base_url =
                            format!("http://{}:{}", speaker_info.address, speaker_info.port);

                        // Get speaker status
                        let params = [
                            ("path", "settings:/kef/host/speakerStatus"),
                            ("roles", "value"),
                        ];

                        if let Ok(response) = client
                            .get(&format!("{}/api/getData", base_url))
                            .query(&params)
                            .send()
                            .await
                        {
                            if let Ok(power_json) = response.json::<serde_json::Value>().await {
                                let power = power_json[0]["kefSpeakerStatus"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_string();

                                let source = if power == "powerOn" {
                                    let params = [
                                        ("path", "settings:/kef/play/physicalSource"),
                                        ("roles", "value"),
                                    ];

                                    if let Ok(response) = client
                                        .get(&format!("{}/api/getData", base_url))
                                        .query(&params)
                                        .send()
                                        .await
                                    {
                                        if let Ok(source_json) =
                                            response.json::<serde_json::Value>().await
                                        {
                                            let kef_source = source_json[0]["kefPhysicalSource"]
                                                .as_str()
                                                .unwrap_or("");
                                            InputSource::from_kef_source(kef_source)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };

                                let status = SpeakerStatus {
                                    power: power.clone(),
                                    source,
                                };
                                debug!("Periodic poll: power={}, source={:?}", power, source);
                                let _ = poll_tx_clone.send(status);
                            }
                        }
                    }
                    interval.tick().await;
                }
            });

            let controller = speaker::SpeakerController::new(internal_rx);
            controller.run().await;
        });
    });

    // Run the UI on the main thread
    menubar::run(tx, poll_rx, speaker_info_rx);
}
