#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::Arc;
use std::time::Duration;

use objc2_foundation::NSString;
use objc2_foundation::ns_string;
use tracing::{debug, info};

// Channel for communication between UI and speaker controller
use tokio::sync::mpsc;
use tokio::sync::oneshot;

mod menubar;
mod speaker;

// Speaker discovery and control commands
#[derive(Debug)]
pub enum SpeakerCommand {
    SetInput(InputSource),
    GetStatus(oneshot::Sender<SpeakerStatus>),
    PowerOn,
    PowerOff,
    PollUpdate(SpeakerStatus),
    // SpeakerDiscovered(SpeakerInfo),
}

#[derive(Debug, Clone)]
pub struct SpeakerInfo {
    pub address: String,
    pub port: u16,
    pub name: String,
    pub model: String,
    pub base_url: String,
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

    fn from_ns_string(s: &NSString) -> Option<Self> {
        if s == ns_string!("USB") {
            Some(InputSource::USB)
        } else if s == ns_string!("WiFi") {
            Some(InputSource::WiFi)
        } else if s == ns_string!("Bluetooth") {
            Some(InputSource::Bluetooth)
        } else if s == ns_string!("Optical") {
            Some(InputSource::Optical)
        } else {
            None
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

fn main() {
    // Initialize tracing first
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting qaf menubar app");

    // The macOS UI thread (main thread) gets the sender; the SpeakerController gets the receiver.
    // Used to communicate between the UI and the http API.
    let (tx, rx) = mpsc::unbounded_channel::<SpeakerCommand>();
    // Speaker status polling task gets the sender. The macOS UI gets the receiver.
    // Used to keep the UI in sync with the state of the speaker.
    let (poll_tx, poll_rx) = mpsc::unbounded_channel::<SpeakerStatus>();

    let speaker_info = speaker::SpeakerController::discover_speaker()
        .expect("no speaker; do something better here");
    let speaker_info2 = Arc::new(tokio::sync::RwLock::new(
        speaker::SpeakerController::discover_speaker()
            .expect("no speaker; do something better here"),
    ));
    let controller = speaker::SpeakerController::new(speaker_info, rx);

    // Spawn the async runtime in a separate thread
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        runtime.block_on(async {
            // Start periodic polling task
            tokio::spawn(async move {
                let client = reqwest::Client::new();
                let mut interval = tokio::time::interval(Duration::from_secs(30));

                loop {
                    // TODO: this re-implements the json API needlessly.
                    // Check if we have discovered a speaker
                    let speaker = speaker_info2.read().await.clone();
                    // Get speaker status
                    let params = [
                        ("path", "settings:/kef/host/speakerStatus"),
                        ("roles", "value"),
                    ];

                    if let Ok(response) = client
                        .get(&format!("{}/api/getData", speaker.base_url))
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
                                    .get(&format!("{}/api/getData", speaker.base_url))
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
                            let _ = poll_tx.send(status);
                        }
                    }
                    interval.tick().await;
                }
            });

            controller.run().await;
        });
    });

    // Run the UI on the main thread
    menubar::run(tx, poll_rx);
}
