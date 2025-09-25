use std::time::Duration;

use crate::{InputSource, SpeakerCommand, SpeakerInfo, SpeakerStatus};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde_json::json;
use tokio::{sync::mpsc, time::sleep};
use tracing::{debug, error, info, trace, warn};

pub struct SpeakerController {
    rx: mpsc::UnboundedReceiver<SpeakerCommand>,
    info: SpeakerInfo,
    client: reqwest::Client,
}

impl SpeakerController {
    pub fn new(info: SpeakerInfo, rx: mpsc::UnboundedReceiver<SpeakerCommand>) -> Self {
        Self {
            rx,
            info,
            client: reqwest::Client::new(),
        }
    }
    pub fn discover_speaker() -> Option<SpeakerInfo> {
        debug!("Starting mDNS discovery for KEF speakers…");
        let service_type = "_kef-info._tcp.local.";
        let mdns = match ServiceDaemon::new() {
            Ok(daemon) => daemon,
            Err(e) => {
                error!("Failed to create mDNS daemon: {}", e);
                return None;
            }
        };

        let receiver = match mdns.browse(service_type) {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to browse for KEF speakers: {}", e);
                return None;
            }
        };
        debug!("Searching for KEF speakers on the network...");
        let mut speaker_info = None;
        while let Ok(event) = receiver.recv() {
            if let ServiceEvent::ServiceResolved(info) = event {
                trace!("Found KEF speaker: {}", info.get_fullname());

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

                    trace!(
                        "KEF Speaker discovered - Name: {}, Model: {}, Address: {}:{}",
                        name, model, addr, port
                    );
                    let address = addr.to_string();
                    speaker_info = Some(SpeakerInfo {
                        address,
                        port,
                        name,
                        model,
                        base_url: format!("http://{}:{}", addr.to_string(), port),
                    });

                    // Stop mDNS discovery by calling shutdown
                    trace!("Stopping mDNS discovery after finding first speaker");
                    drop(receiver);

                    match mdns.shutdown() {
                        Ok(shutdown_rx) => {
                            // Wait for shutdown confirmation
                            if let Ok(_) = shutdown_rx.recv() {
                                trace!("mDNS daemon shutdown successfully");
                            } else {
                                warn!("Failed to receive mDNS shutdown confirmation");
                            }
                        }
                        Err(e) => {
                            warn!("Failed to shutdown mDNS daemon: {}", e);
                        }
                    }
                    break;
                }
            }
        }
        return speaker_info;
    }

    pub async fn run(mut self) {
        debug!("Speaker controller started, waiting for speaker discovery...");

        while let Some(command) = self.rx.recv().await {
            match command {
                SpeakerCommand::SetInput(input) => {
                    debug!("Setting input to: {:?}", input);

                    // First check if we need to power on
                    if let Ok(status) = self.get_speaker_status().await {
                        if status.power == "standby" {
                            info!("Speaker is in standby, powering on first");
                            if let Err(e) = self.power_on().await {
                                error!("Failed to power on: {}", e);
                                continue;
                            }
                            // Wait a bit for the speaker to power on
                            sleep(Duration::from_millis(500)).await;
                        }
                    }

                    if let Err(e) = self.set_input(input).await {
                        error!("Failed to set input: {}", e);
                    }
                }
                SpeakerCommand::GetStatus(tx) => {
                    debug!("Getting speaker status");
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
                    info!("Powering on speakers");
                    if let Err(e) = self.power_on().await {
                        error!("Failed to power on: {}", e);
                    }
                }
                SpeakerCommand::PowerOff => {
                    info!("Powering off speakers");
                    if let Err(e) = self.power_off().await {
                        error!("Failed to power off: {}", e);
                    }
                }
                SpeakerCommand::PollUpdate(status) => {
                    // This is handled by the UI, just log it
                    trace!("Poll update received: {:?}", status);
                }
            }
        }

        info!("Speaker controller shutting down");
    }

    async fn set_input(&self, input: InputSource) -> Result<(), Box<dyn std::error::Error>> {
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
            .get(&format!("{}/api/setData", self.info.base_url))
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
            .get(&format!("{}/api/setData", self.info.base_url))
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
            .get(&format!("{}/api/setData", self.info.base_url))
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
        // Get power status
        let params = [
            ("path", "settings:/kef/host/speakerStatus"),
            ("roles", "value"),
        ];

        let response = self
            .client
            .get(&format!("{}/api/getData", self.info.base_url))
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
                .get(&format!("{}/api/getData", self.info.base_url))
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
