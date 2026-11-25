#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;
mod windivert_ffi;

use eframe::egui;
use engine::{EngineHandle, LiveConfig, Stats};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

// 다크 테마 컬러 팔레트
const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(18, 21, 27);
const CARD_BG: egui::Color32 = egui::Color32::from_rgb(28, 32, 41);
const CARD_BORDER: egui::Color32 = egui::Color32::from_rgb(54, 61, 76);
const INPUT_BG: egui::Color32 = egui::Color32::from_rgb(16, 19, 25);

const ACCENT_BLUE: egui::Color32 = egui::Color32::from_rgb(96, 165, 250);
const ACCENT_GREEN: egui::Color32 = egui::Color32::from_rgb(22, 163, 74);
const ACCENT_RED: egui::Color32 = egui::Color32::from_rgb(220, 38, 38);
const ACCENT_AMBER: egui::Color32 = egui::Color32::from_rgb(251, 191, 36);

const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(236, 240, 247);
const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(148, 163, 184);

fn main() -> eframe::Result {
    let icon_rgba = include_bytes!(concat!(env!("OUT_DIR"), "/icon_64.rgba")).to_vec();
    let icon = Arc::new(egui::IconData {
        rgba: icon_rgba,
        width: 64,
        height: 64,
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 415.0])
            .with_min_inner_size([490.0, 400.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "NetShaper",
        options,
        Box::new(|cc| {
            setup_theme(&cc.egui_ctx);
            Ok(Box::new(NetShaperApp::new()))
        }),
    )
}

fn setup_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);

        let v = &mut style.visuals;
        v.panel_fill = BG_PANEL;
        v.extreme_bg_color = INPUT_BG;
        v.override_text_color = Some(TEXT_PRIMARY);

        // 체크박스/선택 포인트를 선명한 블루로 지정
        v.selection.bg_fill = egui::Color32::from_rgb(59, 130, 246);
        v.selection.stroke = egui::Stroke::new(1.5, egui::Color32::WHITE);

        // 일반 버튼/입력 위젯 대비 강화
        v.widgets.inactive.bg_fill = egui::Color32::from_rgb(40, 46, 58);
        v.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(38, 44, 56);
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(66, 74, 92));

        v.widgets.hovered.bg_fill = egui::Color32::from_rgb(52, 60, 76);
        v.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(50, 58, 74);
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT_BLUE);
    });
}

fn section_card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let frame = egui::Frame::new()
        .fill(CARD_BG)
        .stroke(egui::Stroke::new(1.0, CARD_BORDER))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(14, 10));

    frame.show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        add_contents(ui);
    });
}

struct NetShaperApp {
    filter_text: String,

    lag_enabled: bool,
    lag_inbound: bool,
    lag_outbound: bool,
    lag_ms: u64,

    bw_enabled: bool,
    bw_inbound: bool,
    bw_outbound: bool,
    bw_kbps: u64,

    live_config: Arc<LiveConfig>,
    stats: Arc<Stats>,
    engine: Option<EngineHandle>,
    error_msg: Option<String>,

    last_rate_at: Instant,
    last_rx_bytes: u64,
    last_tx_bytes: u64,
    rx_kbps_rate: f64,
    tx_kbps_rate: f64,
}

impl NetShaperApp {
    fn new() -> Self {
        Self {
            filter_text: "outbound".to_string(),

            lag_enabled: true,
            lag_inbound: true,
            lag_outbound: true,
            lag_ms: 150,

            bw_enabled: true,
            bw_inbound: true,
            bw_outbound: true,
            bw_kbps: 50,

            live_config: Arc::new(LiveConfig::default()),
            stats: Arc::new(Stats::default()),
            engine: None,
            error_msg: None,

            last_rate_at: Instant::now(),
            last_rx_bytes: 0,
            last_tx_bytes: 0,
            rx_kbps_rate: 0.0,
            tx_kbps_rate: 0.0,
        }
    }

    fn sync_config(&self) {
        self.live_config
            .lag_enabled
            .store(self.lag_enabled, Ordering::Relaxed);
        self.live_config
            .lag_inbound
            .store(self.lag_inbound, Ordering::Relaxed);
        self.live_config
            .lag_outbound
            .store(self.lag_outbound, Ordering::Relaxed);
        self.live_config.lag_ms.store(self.lag_ms, Ordering::Relaxed);

        self.live_config
            .bw_enabled
            .store(self.bw_enabled, Ordering::Relaxed);
        self.live_config
            .bw_inbound
            .store(self.bw_inbound, Ordering::Relaxed);
        self.live_config
            .bw_outbound
            .store(self.bw_outbound, Ordering::Relaxed);
        self.live_config
            .bw_kbps
            .store(self.bw_kbps, Ordering::Relaxed);
    }

    fn update_rates(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_rate_at).as_secs_f64();
        if elapsed >= 0.5 {
            let rx_now = self.stats.rx_bytes_total.load(Ordering::Relaxed);
            let tx_now = self.stats.tx_bytes_total.load(Ordering::Relaxed);

            self.rx_kbps_rate =
                (rx_now.saturating_sub(self.last_rx_bytes) as f64) / 1024.0 / elapsed;
            self.tx_kbps_rate =
                (tx_now.saturating_sub(self.last_tx_bytes) as f64) / 1024.0 / elapsed;

            self.last_rx_bytes = rx_now;
            self.last_tx_bytes = tx_now;
            self.last_rate_at = now;
        }
    }
}

impl eframe::App for NetShaperApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let is_running = self.engine.is_some();
        if is_running {
            self.update_rates();
            ui.request_repaint_after(Duration::from_millis(200));
        }

        let panel_frame = egui::Frame::new()
            .fill(BG_PANEL)
            .inner_margin(egui::Margin::same(14));

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ui, |ui| {
                // 1. Filtering 카드
                section_card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("FILTERING")
                                .size(12.0)
                                .strong()
                                .color(ACCENT_BLUE),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if is_running {
                                ui.label(
                                    egui::RichText::new("● Running")
                                        .strong()
                                        .color(egui::Color32::from_rgb(74, 222, 128)),
                                );
                            } else {
                                ui.label(egui::RichText::new("● Stopped").color(TEXT_MUTED));
                            }
                        });
                    });
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Filter:").color(TEXT_MUTED));
                        ui.add_enabled(
                            !is_running,
                            egui::TextEdit::singleline(&mut self.filter_text)
                                .desired_width(ui.available_width() - 98.0),
                        );

                        let (btn_text, btn_color) = if is_running {
                            ("Stop", ACCENT_RED)
                        } else {
                            ("Start", ACCENT_GREEN)
                        };
                        let btn = egui::Button::new(
                            egui::RichText::new(btn_text)
                                .strong()
                                .color(egui::Color32::WHITE),
                        )
                        .fill(btn_color);

                        if ui.add_sized([86.0, 26.0], btn).clicked() {
                            if let Some(mut handle) = self.engine.take() {
                                handle.stop();
                                self.rx_kbps_rate = 0.0;
                                self.tx_kbps_rate = 0.0;
                                self.error_msg = None;
                            } else {
                                self.sync_config();
                                match EngineHandle::start(
                                    &self.filter_text,
                                    Arc::clone(&self.live_config),
                                    Arc::clone(&self.stats),
                                ) {
                                    Ok(handle) => {
                                        self.engine = Some(handle);
                                        self.error_msg = None;
                                        self.last_rx_bytes = 0;
                                        self.last_tx_bytes = 0;
                                        self.last_rate_at = Instant::now();
                                    }
                                    Err(err) => {
                                        self.error_msg = Some(err);
                                    }
                                }
                            }
                        }
                    });

                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Presets:").color(TEXT_MUTED));
                        ui.add_enabled_ui(!is_running, |ui| {
                            for (name, expr) in [
                                ("outbound", "outbound"),
                                ("inbound", "inbound"),
                                ("both", "outbound or inbound"),
                            ] {
                                let selected = self.filter_text.trim() == expr;
                                let mut btn = egui::Button::new(
                                    egui::RichText::new(name).size(12.0).color(if selected {
                                        egui::Color32::WHITE
                                    } else {
                                        TEXT_PRIMARY
                                    }),
                                )
                                .small();
                                if selected {
                                    btn = btn.fill(egui::Color32::from_rgb(59, 130, 246));
                                }
                                if ui.add(btn).clicked() {
                                    self.filter_text = expr.to_string();
                                }
                            }
                        });
                    });
                });

                if let Some(err) = &self.error_msg {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(248, 113, 113), err);
                }

                ui.add_space(10.0);

                // 2. Functions 카드
                let mut changed = false;
                section_card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("FUNCTIONS")
                                .size(12.0)
                                .strong()
                                .color(ACCENT_BLUE),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new("실행 중 실시간 반영")
                                    .size(11.5)
                                    .color(TEXT_MUTED),
                            );
                        });
                    });
                    ui.add_space(4.0);

                    // Lag 행
                    ui.horizontal(|ui| {
                        ui.allocate_ui(egui::vec2(102.0, 22.0), |ui| {
                            let lbl = egui::RichText::new("Lag").strong().color(if self.lag_enabled {
                                TEXT_PRIMARY
                            } else {
                                TEXT_MUTED
                            });
                            changed |= ui.checkbox(&mut self.lag_enabled, lbl).changed();
                        });
                        ui.allocate_ui(egui::vec2(78.0, 22.0), |ui| {
                            changed |= ui.checkbox(&mut self.lag_inbound, "Inbound").changed();
                        });
                        ui.allocate_ui(egui::vec2(86.0, 22.0), |ui| {
                            changed |= ui.checkbox(&mut self.lag_outbound, "Outbound").changed();
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut self.lag_ms)
                                        .range(0..=10_000)
                                        .speed(1.0)
                                        .suffix(" ms"),
                                )
                                .changed();
                            ui.label(egui::RichText::new("Delay:").color(TEXT_MUTED));
                        });
                    });

                    ui.separator();

                    // Bandwidth (Token Bucket) 행
                    ui.horizontal(|ui| {
                        ui.allocate_ui(egui::vec2(102.0, 22.0), |ui| {
                            let lbl = egui::RichText::new("Bandwidth").strong().color(
                                if self.bw_enabled {
                                    TEXT_PRIMARY
                                } else {
                                    TEXT_MUTED
                                },
                            );
                            changed |= ui.checkbox(&mut self.bw_enabled, lbl).changed();
                        });
                        ui.allocate_ui(egui::vec2(78.0, 22.0), |ui| {
                            changed |= ui.checkbox(&mut self.bw_inbound, "Inbound").changed();
                        });
                        ui.allocate_ui(egui::vec2(86.0, 22.0), |ui| {
                            changed |= ui.checkbox(&mut self.bw_outbound, "Outbound").changed();
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mbps = (self.bw_kbps as f64) * 8.0 / 1000.0;
                            ui.label(
                                egui::RichText::new(format!("({:.2} Mbps)", mbps))
                                    .size(12.0)
                                    .color(TEXT_MUTED),
                            );
                            changed |= ui
                                .add(
                                    egui::DragValue::new(&mut self.bw_kbps)
                                        .range(0..=100_000)
                                        .speed(5.0)
                                        .suffix(" KB/s"),
                                )
                                .changed();
                            ui.label(egui::RichText::new("Limit:").color(TEXT_MUTED));
                        });
                    });
                });

                if changed {
                    self.sync_config();
                }

                ui.add_space(10.0);

                // 3. Statistics 카드
                section_card(ui, |ui| {
                    ui.label(
                        egui::RichText::new("STATISTICS")
                            .size(12.0)
                            .strong()
                            .color(ACCENT_BLUE),
                    );
                    ui.add_space(4.0);

                    let rx = self.stats.rx_packets.load(Ordering::Relaxed);
                    let tx = self.stats.tx_packets.load(Ordering::Relaxed);
                    let q = self.stats.queued_packets.load(Ordering::Relaxed);
                    let drop = self.stats.dropped_packets.load(Ordering::Relaxed);

                    egui::Grid::new("stats_grid")
                        .num_columns(2)
                        .spacing([40.0, 6.0])
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("RX:").color(TEXT_MUTED));
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} pkts ({:.1} KB/s)",
                                        rx, self.rx_kbps_rate
                                    ))
                                    .strong(),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("TX:").color(TEXT_MUTED));
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} pkts ({:.1} KB/s)",
                                        tx, self.tx_kbps_rate
                                    ))
                                    .strong(),
                                );
                            });
                            ui.end_row();

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Queued:").color(TEXT_MUTED));
                                let q_color = if q > 0 { ACCENT_AMBER } else { TEXT_PRIMARY };
                                ui.label(
                                    egui::RichText::new(format!("{} pkts", q))
                                        .strong()
                                        .color(q_color),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Dropped:").color(TEXT_MUTED));
                                let d_color = if drop > 0 {
                                    egui::Color32::from_rgb(248, 113, 113)
                                } else {
                                    TEXT_PRIMARY
                                };
                                ui.label(
                                    egui::RichText::new(format!("{} pkts", drop))
                                        .strong()
                                        .color(d_color),
                                );
                            });
                            ui.end_row();
                        });
                });
            });
    }
}