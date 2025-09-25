#![allow(unsafe_op_in_unsafe_fn)]

use std::sync::Arc;
use std::time::Duration;

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
    SpeakerDiscovered(SpeakerInfo),
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

fn main() {
    // Initialize tracing first
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env(), // .add_directive(tracing::Level::DEBUG.into()),
        )
        .init();

    info!("Starting qaf menubar app");

    // The macOS UI thread (main thread) gets the sender; the SpeakerController gets the receiver.
    let (tx, rx) = mpsc::unbounded_channel::<SpeakerCommand>();
    let (poll_tx, poll_rx) = mpsc::unbounded_channel::<SpeakerStatus>();

    let poll_tx_clone = poll_tx.clone();

    let speaker_info = Arc::new(tokio::sync::RwLock::new(
        speaker::SpeakerController::discover_speaker()
            .expect("no speaker; do something better here"),
    ));
    let speaker_info2 = speaker_info.clone();

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
                    let speaker = speaker_info.read().await.clone();
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
                            let _ = poll_tx_clone.send(status);
                        }
                    }
                    interval.tick().await;
                }
            });

            let controller = speaker::SpeakerController::new(rx);
            controller.run().await;
        });
    });

    // Run the UI on the main thread
    menubar::run(tx, poll_rx, speaker_info2);
}
