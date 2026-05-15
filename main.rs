use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use tiny_http::{Header, Response, Server};

// Konstanta Sistem
const WINDOW_SIZE: usize   = 5;      // Ukuran jendela moving average
const LEAK_WARN_MA: f32    = 15.0;   // Batas bawah WASPADA (mA)
const LEAK_DANGER_MA: f32  = 22.0;   // Batas bawah BAHAYA (mA)
const VOLTAGE_NOMINAL: f32 = 220.0;  // Tegangan nominal PLN (V)
const MAX_CURRENT_MA: f32  = 20_000.0; // Batas maksimum arus sensor (mA)

// Status Kebocoran
#[derive(Debug, PartialEq, Clone)]
enum LeakStatus { Aman, Waspada, Bahaya }

impl LeakStatus {
    fn from_ma(ma: f32) -> Self {
        if ma >= LEAK_DANGER_MA     { LeakStatus::Bahaya }
        else if ma >= LEAK_WARN_MA  { LeakStatus::Waspada }
        else                        { LeakStatus::Aman }
    }
    fn label(&self) -> &str {
        match self {
            Self::Aman    => "AMAN",
            Self::Waspada => "WASPADA",
            Self::Bahaya  => "BAHAYA",
        }
    }
    fn color(&self) -> &str {
        match self {
            Self::Aman    => "#16a34a",
            Self::Waspada => "#d97706",
            Self::Bahaya  => "#dc2626",
        }
    }
}

// Aktuator — LED dan Buzzer (output fisik ESP32)
struct Actuator {
    led_hijau:  bool,
    led_kuning: bool,
    led_merah:  bool,
    buzzer:     bool,
}

impl Actuator {
    fn new() -> Self {
        Actuator { led_hijau: true, led_kuning: false, led_merah: false, buzzer: false }
    }

    fn update(&mut self, status: &LeakStatus) {
        match status {
            LeakStatus::Aman    => { self.led_hijau=true;  self.led_kuning=false; self.led_merah=false; self.buzzer=false; }
            LeakStatus::Waspada => { self.led_hijau=false; self.led_kuning=true;  self.led_merah=false; self.buzzer=false; }
            LeakStatus::Bahaya  => { self.led_hijau=false; self.led_kuning=false; self.led_merah=true;  self.buzzer=true;  }
        }
    }
}

// Sensor — Satu Cabang MCB
struct Sensor {
    name:              String,
    voltage:           f32,   // Tegangan dari ADC (V)
    current_fasa_ma:   f32,   // Arus fasa dari SCT-013 (mA)
    current_netral_ma: f32,   // Arus netral dari SCT-013 (mA)
    leak_ma:           f32,   // Arus bocor hasil kalkulasi (mA)
    history_fasa:      VecDeque<f32>, // Riwayat arus fasa untuk MA-5
    is_valid:          bool,  // Status validitas data sensor
    actuator:          Actuator,
}

impl Sensor {
    fn new(name: &str) -> Self {
        Sensor {
            name:              name.to_string(),
            voltage:           VOLTAGE_NOMINAL,
            current_fasa_ma:   0.0,
            current_netral_ma: 0.0,
            leak_ma:           0.0,
            history_fasa:      VecDeque::with_capacity(WINDOW_SIZE),
            is_valid:          true,
            actuator:          Actuator::new(),
        }
    }

    fn power_watt(&self) -> f32 {
        self.voltage * (self.current_fasa_ma / 1000.0)
    }

    fn validate(&mut self) {
        self.is_valid = self.voltage >= 100.0
            && self.voltage <= 260.0
            && self.current_fasa_ma >= 0.0
            && self.current_fasa_ma <= MAX_CURRENT_MA
            && self.current_netral_ma >= 0.0
            && self.current_netral_ma <= MAX_CURRENT_MA;
    }

    fn compute_leak(&mut self) {
        self.leak_ma = (self.current_fasa_ma - self.current_netral_ma).max(0.0);
    }

    fn push_history(&mut self) {
        if self.history_fasa.len() == WINDOW_SIZE { self.history_fasa.pop_front(); }
        self.history_fasa.push_back(self.current_fasa_ma);
    }

    fn moving_average(&self) -> f32 {
        if self.history_fasa.is_empty() { return 0.0; }
        self.history_fasa.iter().sum::<f32>() / self.history_fasa.len() as f32
    }

    fn std_deviation(&self) -> f32 {
        if self.history_fasa.len() < 2 { return 0.0; }
        let avg = self.moving_average();
        let var: f32 = self.history_fasa.iter()
            .map(|x| (x - avg).powi(2))
            .sum::<f32>()
            / (self.history_fasa.len() as f32 - 1.0);
        var.sqrt()
    }

    fn status(&self) -> LeakStatus {
        if !self.is_valid { return LeakStatus::Bahaya; }
        LeakStatus::from_ma(self.leak_ma)
    }

    fn update_actuator(&mut self) {
        let st = self.status();
        self.actuator.update(&st);
    }
}

// Controller — Logika Keputusan Per Cabang
struct Controller {
    name:           String,
    total_warnings: u32,
    total_dangers:  u32,
}

impl Controller {
    fn new(name: &str) -> Self {
        Controller { name: name.to_string(), total_warnings: 0, total_dangers: 0 }
    }

    fn process(&mut self, status: &LeakStatus) {
        match status {
            LeakStatus::Waspada => self.total_warnings += 1,
            LeakStatus::Bahaya  => self.total_dangers  += 1,
            _ => {}
        }
    }

    fn action_message(&self, status: &LeakStatus) -> String {
        match status {
            LeakStatus::Aman    => format!("MCB {} normal.", self.name),
            LeakStatus::Waspada => format!("Pantau MCB {}! Ada indikasi kebocoran.", self.name),
            LeakStatus::Bahaya  => format!("SEGERA MATIKAN MCB {}! Cek instalasi!", self.name),
        }
    }
}

// Sistem Monitoring Utama
struct MonitoringSystem {
    sensors:          Vec<Sensor>,
    controllers:      Vec<Controller>,
    total_energy_kwh: f32,
    reading_count:    u32,
}

impl MonitoringSystem {
    fn new() -> Self {
        MonitoringSystem {
            sensors: vec![
                Sensor::new("DAPUR"),
                Sensor::new("KAMAR"),
                Sensor::new("R.TAMU"),
            ],
            controllers: vec![
                Controller::new("DAPUR"),
                Controller::new("KAMAR"),
                Controller::new("R.TAMU"),
            ],
            total_energy_kwh: 0.0,
            reading_count:    0,
        }
    }

    fn total_power(&self) -> f32 {
        self.sensors.iter().map(|s| s.power_watt()).sum()
    }

    fn dominant_branch(&self) -> String {
        self.sensors.iter()
            .max_by(|a, b| a.power_watt().partial_cmp(&b.power_watt()).unwrap())
            .map(|s| s.name.clone()).unwrap_or("-".to_string())
    }

    fn update_energy(&mut self) {
        self.total_energy_kwh += (self.total_power() / 1000.0) * 1.0;
        self.reading_count    += 1;
    }

    fn update_all(&mut self) {
        for s in self.sensors.iter_mut() {
            s.validate();
            s.compute_leak();
            s.push_history();
            s.update_actuator();
        }
        for i in 0..self.sensors.len() {
            let st = self.sensors[i].status();
            self.controllers[i].process(&st);
        }
        self.update_energy();
    }

    fn overall_status(&self) -> LeakStatus {
        if self.sensors.iter().any(|s| s.status() == LeakStatus::Bahaya)       { LeakStatus::Bahaya }
        else if self.sensors.iter().any(|s| s.status() == LeakStatus::Waspada) { LeakStatus::Waspada }
        else                                                                     { LeakStatus::Aman }
    }
}

// Render Dashboard HTML
fn render_dashboard(sys: &MonitoringSystem) -> String {
    let overall  = sys.overall_status();
    let ov_color = overall.color();
    let ov_label = overall.label();
    let ov_pulse = if overall == LeakStatus::Bahaya { "pulse-badge" } else { "" };

    let sensor_cards: String = sys.sensors.iter().zip(sys.controllers.iter()).map(|(s, c)| {
        let st     = s.status();
        let color  = st.color();
        let label  = st.label();
        let action = c.action_message(&st);
        let vclr   = if s.is_valid { "#16a34a" } else { "#dc2626" };
        let vtxt   = if s.is_valid { "Valid" }   else { "ERROR" };

            let (lh, lhs) = if s.actuator.led_hijau  { ("#16a34a","0 0 8px #16a34a88") } else { ("#d1d5db","none") };
        let (lk, lks) = if s.actuator.led_kuning { ("#d97706","0 0 8px #d9770688") } else { ("#d1d5db","none") };
        let (lm, lms) = if s.actuator.led_merah  { ("#dc2626","0 0 8px #dc262688") } else { ("#d1d5db","none") };
        let (bc, bcs, ba) = if s.actuator.buzzer { ("#dc2626","0 0 8px #dc262688","buz-anim") } else { ("#e5e7eb","none","") };

            let bars: String = s.history_fasa.iter().map(|v| {
            let pct = (v / MAX_CURRENT_MA * 2000.0).min(100.0);
            format!(r#"<div style="flex:1;background:{color};border-radius:3px;min-height:3px;height:{pct:.0}%"></div>"#)
        }).collect();

        format!(r#"<div style="background:#ffffff;border-radius:14px;padding:20px;border-top:4px solid {color};border:1px solid #e5e7eb;border-top:4px solid {color};box-shadow:0 1px 4px #0000000d">
          <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:14px">
            <div>
              <div style="font-size:15px;font-weight:700;color:#111827">MCB {name}</div>
              <div style="font-size:10px;color:{vclr};margin-top:2px">● Sensor {vtxt}</div>
            </div>
            <span style="background:{color};color:#fff;padding:4px 12px;border-radius:999px;font-size:11px;font-weight:700">{label}</span>
          </div>

          <div style="display:grid;grid-template-columns:1fr 1fr;gap:8px;margin-bottom:8px">
            <div style="background:#f8fafc;border-radius:8px;padding:9px;text-align:center;border:1px solid #e5e7eb">
              <div style="font-size:16px;font-weight:700;color:#111827">{volt:.0}<span style="font-size:10px;color:#6b7280"> V</span></div>
              <div style="font-size:10px;color:#6b7280">Tegangan</div>
            </div>
            <div style="background:#f8fafc;border-radius:8px;padding:9px;text-align:center;border:1px solid #e5e7eb">
              <div style="font-size:16px;font-weight:700;color:#111827">{pow:.1}<span style="font-size:10px;color:#6b7280"> W</span></div>
              <div style="font-size:10px;color:#6b7280">Daya</div>
            </div>
          </div>

          <div style="background:#f8fafc;border-radius:10px;padding:12px;margin-bottom:8px;border:1px solid #e5e7eb">
            <div style="font-size:9px;color:#9ca3af;margin-bottom:8px;letter-spacing:.4px">KALKULASI ARUS BOCOR  ·  I_bocor = I_fasa − I_netral</div>
            <div style="display:flex;align-items:center;gap:5px">
              <div style="flex:1;background:#eff6ff;border-radius:8px;padding:9px;text-align:center;border:1px solid #bfdbfe">
                <div style="font-size:9px;color:#2563eb;font-weight:600;margin-bottom:3px">I FASA</div>
                <div style="font-size:18px;font-weight:700;color:#1d4ed8">{fasa:.1}</div>
                <div style="font-size:9px;color:#6b7280">mA</div>
              </div>
              <div style="text-align:center;min-width:20px;color:#9ca3af;font-size:14px">−</div>
              <div style="flex:1;background:#f8fafc;border-radius:8px;padding:9px;text-align:center;border:1px solid #e5e7eb">
                <div style="font-size:9px;color:#6b7280;font-weight:600;margin-bottom:3px">I NETRAL</div>
                <div style="font-size:18px;font-weight:700;color:#374151">{netral:.1}</div>
                <div style="font-size:9px;color:#6b7280">mA</div>
              </div>
              <div style="text-align:center;min-width:20px;color:#9ca3af;font-size:14px">=</div>
              <div style="flex:1;background:#fff;border-radius:8px;padding:9px;text-align:center;border:2px solid {color}">
                <div style="font-size:9px;color:{color};font-weight:600;margin-bottom:3px">I BOCOR</div>
                <div style="font-size:18px;font-weight:700;color:{color}">{leak:.2}</div>
                <div style="font-size:9px;color:#6b7280">mA</div>
              </div>
            </div>
          </div>

          <div style="background:#f8fafc;border-radius:8px;padding:8px 12px;margin-bottom:8px;display:flex;justify-content:space-between;border:1px solid #e5e7eb">
            <span style="font-size:10px;color:#6b7280">MA-5 (I fasa)</span>
            <span style="font-size:13px;font-weight:700;color:#2563eb">{ma:.1} mA</span>
            <span style="font-size:10px;color:#6b7280;margin-left:12px">σ</span>
            <span style="font-size:13px;font-weight:700;color:#7c3aed">{sd:.3}</span>
          </div>

          <div style="display:flex;justify-content:space-around;background:#f8fafc;border-radius:8px;padding:9px;margin-bottom:8px;border:1px solid #e5e7eb">
            <div style="text-align:center">
              <div style="width:14px;height:14px;border-radius:50%;background:{lh};box-shadow:{lhs};margin:0 auto 3px"></div>
              <div style="font-size:9px;color:#6b7280">Hijau</div>
            </div>
            <div style="text-align:center">
              <div style="width:14px;height:14px;border-radius:50%;background:{lk};box-shadow:{lks};margin:0 auto 3px"></div>
              <div style="font-size:9px;color:#6b7280">Kuning</div>
            </div>
            <div style="text-align:center">
              <div style="width:14px;height:14px;border-radius:50%;background:{lm};box-shadow:{lms};margin:0 auto 3px"></div>
              <div style="font-size:9px;color:#6b7280">Merah</div>
            </div>
            <div style="text-align:center" class="{ba}">
              <div style="width:14px;height:14px;border-radius:50%;background:{bc};box-shadow:{bcs};margin:0 auto 3px"></div>
              <div style="font-size:9px;color:#6b7280">Buzzer</div>
            </div>
          </div>

          <div style="display:flex;gap:3px;align-items:flex-end;height:32px;background:#f8fafc;border-radius:6px;padding:4px 8px;margin-bottom:3px;border:1px solid #e5e7eb">{bars}</div>
          <div style="font-size:9px;color:#9ca3af;text-align:center;margin-bottom:10px">Riwayat I fasa (MA-5)</div>

          <div style="font-size:11px;color:{color};background:{color}18;border-radius:6px;padding:7px 10px;border-left:3px solid {color}">{action}</div>
          <div style="font-size:10px;color:#9ca3af;margin-top:5px">Waspada: {warn}× &nbsp;|&nbsp; Bahaya: {dang}×</div>
        </div>"#,
            color=color, name=s.name, label=label, vclr=vclr, vtxt=vtxt,
            volt=s.voltage, pow=s.power_watt(),
            fasa=s.current_fasa_ma, netral=s.current_netral_ma, leak=s.leak_ma,
            ma=s.moving_average(), sd=s.std_deviation(),
            lh=lh, lhs=lhs, lk=lk, lks=lks, lm=lm, lms=lms,
            bc=bc, bcs=bcs, ba=ba, bars=bars, action=action,
            warn=c.total_warnings, dang=c.total_dangers,
        )
    }).collect();

    let alarm_html = if overall != LeakStatus::Aman {
        let rows: String = sys.sensors.iter().zip(sys.controllers.iter())
            .filter(|(s,_)| s.status() != LeakStatus::Aman)
            .map(|(s,c)| {
                let st  = s.status();
                let act = c.action_message(&st);
                format!(r#"<tr style="border-bottom:1px solid #f3f4f6">
                  <td style="padding:8px 12px;color:{clr};font-weight:700">{name}</td>
                  <td style="padding:8px 12px;color:#2563eb">{fasa:.1} mA</td>
                  <td style="padding:8px 12px;color:#374151">{netral:.1} mA</td>
                  <td style="padding:8px 12px;color:{clr};font-weight:700">{leak:.2} mA</td>
                  <td style="padding:8px 12px;color:{clr}">{lbl}</td>
                  <td style="padding:8px 12px;color:#374151">{act}</td>
                </tr>"#,
                    clr=st.color(), name=s.name,
                    fasa=s.current_fasa_ma, netral=s.current_netral_ma,
                    leak=s.leak_ma, lbl=st.label(), act=act)
            }).collect();
        format!(r#"<div style="margin:0 28px 20px;background:#fff;border:1px solid #fca5a5;border-radius:12px;padding:18px;box-shadow:0 1px 4px #0000000d">
          <div style="color:#dc2626;font-size:14px;font-weight:700;margin-bottom:12px">⚠ Peringatan — Terdeteksi Kebocoran Arus</div>
          <div style="overflow-x:auto"><table style="width:100%;border-collapse:collapse;font-size:12px">
            <tr style="color:#9ca3af;border-bottom:2px solid #e5e7eb;background:#f8fafc">
              <th style="padding:6px 12px;text-align:left">Cabang</th>
              <th style="padding:6px 12px;text-align:left">I Fasa</th>
              <th style="padding:6px 12px;text-align:left">I Netral</th>
              <th style="padding:6px 12px;text-align:left">I Bocor</th>
              <th style="padding:6px 12px;text-align:left">Status</th>
              <th style="padding:6px 12px;text-align:left">Tindakan</th>
            </tr>{rows}
          </table></div>
        </div>"#, rows=rows)
    } else {
        r#"<div style="margin:0 28px 20px;background:#f0fdf4;border:1px solid #86efac;border-radius:12px;padding:14px;color:#16a34a;font-weight:600;font-size:13px">
          ✓ Semua cabang AMAN — Tidak terdeteksi kebocoran arus
        </div>"#.to_string()
    };

    format!(r#"<!DOCTYPE html>
<html lang="id">
<head>
<meta charset="UTF-8">
<meta http-equiv="refresh" content="3">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>ELISA Dashboard</title>
<style>
  *{{box-sizing:border-box;margin:0;padding:0}}
  body{{font-family:'Segoe UI',system-ui,sans-serif;background:#f1f5f9;color:#111827;min-height:100vh}}
  @keyframes pulse-badge{{0%,100%{{opacity:1;transform:scale(1)}}50%{{opacity:.8;transform:scale(1.04)}}}}
  @keyframes buz-anim{{0%,100%{{opacity:1}}50%{{opacity:.2}}}}
  .pulse-badge{{animation:pulse-badge .8s ease-in-out infinite}}
  .buz-anim{{animation:buz-anim .5s ease-in-out infinite}}
  ::-webkit-scrollbar{{width:5px;height:5px}}
  ::-webkit-scrollbar-track{{background:#f1f5f9}}
  ::-webkit-scrollbar-thumb{{background:#cbd5e1;border-radius:3px}}
</style>
</head>
<body>

<!-- Header -->
<div style="background:#ffffff;padding:16px 28px;display:flex;align-items:center;justify-content:space-between;border-bottom:1px solid #e5e7eb;position:sticky;top:0;z-index:10;box-shadow:0 1px 4px #0000000d">
  <div>
    <div style="font-size:19px;font-weight:700;color:#1d4ed8">⚡ ELISA
      <span style="color:#9ca3af;font-size:11px;font-weight:400;margin-left:8px">Electrical Monitoring &amp; Intelligent Safety Alarm</span>
    </div>
    <div style="font-size:10px;color:#9ca3af;margin-top:2px">Teknik Instrumentasi — ITS Surabaya &nbsp;·&nbsp; Sensor: SCT-013 (CT Clamp Non-Invasif)</div>
  </div>
  <div style="text-align:right">
    <div style="display:inline-block;background:{ov_color};color:#fff;padding:5px 16px;border-radius:999px;font-size:12px;font-weight:700;letter-spacing:1px" class="{ov_pulse}">{ov_label}</div>
    <div style="font-size:10px;color:#9ca3af;margin-top:4px">Pembacaan ke-{rc} &nbsp;·&nbsp; refresh/3 dtk</div>
  </div>
</div>

<!-- Summary bar -->
<div style="display:grid;grid-template-columns:repeat(4,1fr);gap:12px;padding:18px 28px">
  <div style="background:#ffffff;border-radius:10px;padding:12px 16px;border:1px solid #e5e7eb;box-shadow:0 1px 4px #0000000d">
    <div style="font-size:20px;font-weight:700;color:#1d4ed8">{tp:.1} W</div>
    <div style="font-size:10px;color:#6b7280;margin-top:2px">Total Daya</div>
  </div>
  <div style="background:#ffffff;border-radius:10px;padding:12px 16px;border:1px solid #e5e7eb;box-shadow:0 1px 4px #0000000d">
    <div style="font-size:20px;font-weight:700;color:#1d4ed8">{te:.4} kWh</div>
    <div style="font-size:10px;color:#6b7280;margin-top:2px">Akumulasi Energi</div>
  </div>
  <div style="background:#ffffff;border-radius:10px;padding:12px 16px;border:1px solid #e5e7eb;box-shadow:0 1px 4px #0000000d">
    <div style="font-size:20px;font-weight:700;color:#d97706">{dom}</div>
    <div style="font-size:10px;color:#6b7280;margin-top:2px">Cabang Dominan</div>
  </div>
  <div style="background:#ffffff;border-radius:10px;padding:12px 16px;border:1px solid #e5e7eb;box-shadow:0 1px 4px #0000000d">
    <div style="font-size:13px;font-weight:700;color:#7c3aed">≥{wt:.0} mA WASPADA &nbsp;·&nbsp; ≥{dt:.0} mA BAHAYA</div>
    <div style="font-size:10px;color:#6b7280;margin-top:2px">Threshold Kebocoran</div>
  </div>
</div>

<!-- Legenda -->
<div style="display:flex;gap:14px;padding:0 28px 14px;flex-wrap:wrap;font-size:10px;color:#9ca3af">
  <span>● <span style="color:#16a34a">Hijau</span> = AMAN (&lt;{wt:.0} mA)</span>
  <span>● <span style="color:#d97706">Kuning</span> = WASPADA ({wt:.0}–{dt:.0} mA)</span>
  <span>● <span style="color:#dc2626">Merah+Buzzer</span> = BAHAYA (&gt;{dt:.0} mA)</span>
  <span>· I_bocor = I_fasa − I_netral (prinsip SCT-013)</span>
</div>

<!-- Grid Kartu Sensor -->
<div style="display:grid;grid-template-columns:repeat(3,1fr);gap:16px;padding:0 28px 20px">
  {sensor_cards}
</div>

<!-- Blok Alarm / Aman -->
{alarm_html}

<!-- Footer -->
<div style="text-align:center;color:#d1d5db;font-size:10px;padding:12px;border-top:1px solid #e5e7eb">
  ELISA v2.0 &mdash; ETS Algoritma dan Pemrograman &mdash; Teknik Instrumentasi ITS
</div>
</body>
</html>"#,
        ov_color=ov_color, ov_label=ov_label, ov_pulse=ov_pulse,
        rc=sys.reading_count, tp=sys.total_power(), te=sys.total_energy_kwh,
        dom=sys.dominant_branch(), wt=LEAK_WARN_MA, dt=LEAK_DANGER_MA,
        sensor_cards=sensor_cards, alarm_html=alarm_html,
    )
}

// Helper Input Terminal
fn baca_f32(prompt: &str) -> f32 {
    loop {
        print!("{}", prompt); io::stdout().flush().unwrap();
        let mut b = String::new(); io::stdin().read_line(&mut b).unwrap();
        match b.trim().parse::<f32>() { Ok(v) => return v, Err(_) => println!("  [!] Masukkan angka.") }
    }
}
fn baca_usize(prompt: &str) -> usize {
    loop {
        print!("{}", prompt); io::stdout().flush().unwrap();
        let mut b = String::new(); io::stdin().read_line(&mut b).unwrap();
        match b.trim().parse::<usize>() { Ok(v) => return v, Err(_) => println!("  [!] Masukkan angka bulat.") }
    }
}
fn baca_yn(prompt: &str) -> bool {
    loop {
        print!("{}", prompt); io::stdout().flush().unwrap();
        let mut b = String::new(); io::stdin().read_line(&mut b).unwrap();
        match b.trim().to_lowercase().as_str() {
            "y"|"ya" => return true, "n"|"tidak" => return false,
            _ => println!("  [!] y / n")
        }
    }
}

// Input Manual
fn input_manual(sys: &mut MonitoringSystem) {
    println!("\n  ── INPUT MANUAL ─────────────────────────────");
    for s in sys.sensors.iter_mut() {
        println!("  MCB {}", s.name);
        s.voltage           = baca_f32("    Tegangan (V)     : ");
        s.current_fasa_ma   = baca_f32("    I fasa (mA)      : ");
        s.current_netral_ma = baca_f32("    I netral (mA)    : ");
        s.validate();
        if !s.is_valid { println!("  [!] Data tidak valid — sensor error."); }
        else { println!("    → I_bocor = {:.1} - {:.1} = {:.2} mA  |  {:.1} W\n",
            s.current_fasa_ma, s.current_netral_ma,
            (s.current_fasa_ma - s.current_netral_ma).max(0.0), s.power_watt()); }
    }
}

// Simulasi Otomatis
// Profil: DAPUR (kulkas+microwave), KAMAR (AC+charger), R.TAMU (TV+lampu)
fn input_simulasi(sys: &mut MonitoringSystem, idx: u32) {
    let profiles: [(f32, f32); 3] = [
        (220.0, 800.0),
        (220.0, 500.0),
        (220.0, 350.0),
    ];
    for (i, s) in sys.sensors.iter_mut().enumerate() {
        let (v, i_base) = profiles[i];
        let nv = ((idx*7 + i as u32*13) % 10) as f32 * 2.0 - 10.0;
        let ni = ((idx*3 + i as u32*17) % 20) as f32 * 5.0 - 50.0;
        s.voltage           = (v + nv).clamp(210.0, 230.0);
        s.current_fasa_ma   = (i_base + ni).clamp(50.0, 5000.0);
        s.is_valid          = true;

        let seed = (idx * 31 + i as u32 * 17 + idx * i as u32 * 7) % 100;
        let leak = if seed < 50 {
            (seed % 14) as f32 * 0.8    // 50% → AMAN
        } else if seed < 75 {
            15.0 + (seed % 7) as f32    // 25% → WASPADA
        } else {
            23.0 + (seed % 15) as f32   // 25% → BAHAYA
        };
        s.current_netral_ma = (s.current_fasa_ma - leak).max(0.0);
    }
}

// Injeksi Kebocoran — Demo Skenario
// Opsi 1/2: ubah satu cabang saja, cabang lain tetap.
// Opsi 3/4: reset satu atau semua cabang ke AMAN.
fn inject_kebocoran(sys: &mut MonitoringSystem) {
    println!("\n  ── INJEKSI KEBOCORAN ────────────────────────");

    println!("  Status cabang saat ini:");
    for s in sys.sensors.iter() {
        let leak = (s.current_fasa_ma - s.current_netral_ma).max(0.0);
        let st   = LeakStatus::from_ma(leak);
        println!("    · MCB {} — I_bocor {:.1} mA ({})", s.name, leak, st.label());
    }
    println!();
    println!("  1. WASPADA  — injeksi ~18 mA pada cabang pilihan");
    println!("  2. BAHAYA   — injeksi ~30 mA pada cabang pilihan");
    println!("  3. Reset cabang tertentu ke AMAN");
    println!("  4. Reset semua cabang ke AMAN\n");

    let sken = baca_usize("  Pilih [1-4]: ");

    if sken == 4 {
        for s in sys.sensors.iter_mut() {
            s.current_netral_ma = (s.current_fasa_ma - 1.0).max(0.0);
        }
        println!("  [OK] Semua cabang direset ke AMAN.");
        return;
    }

    println!("  Pilih cabang:");
    for (i, s) in sys.sensors.iter().enumerate() { println!("    {}. {}", i+1, s.name); }
    let cb = baca_usize("  Nomor: ");
    if cb < 1 || cb > sys.sensors.len() { println!("  [!] Tidak valid."); return; }
    let idx = cb - 1;

    match sken {
        1 => {
                    sys.sensors[idx].current_netral_ma =
                (sys.sensors[idx].current_fasa_ma - 18.0).max(0.0);
            println!("  [OK] MCB {} → I_bocor ~18 mA (WASPADA).", sys.sensors[idx].name);
            println!("       Cabang lain tidak berubah.");
        }
        2 => {
                    sys.sensors[idx].current_netral_ma =
                (sys.sensors[idx].current_fasa_ma - 30.0).max(0.0);
            println!("  [OK] MCB {} → I_bocor ~30 mA (BAHAYA).", sys.sensors[idx].name);
            println!("       Cabang lain tidak berubah.");
        }
        3 => {
                    sys.sensors[idx].current_netral_ma =
                (sys.sensors[idx].current_fasa_ma - 1.0).max(0.0);
            println!("  [OK] MCB {} direset ke AMAN.", sys.sensors[idx].name);
        }
        _ => println!("  [!] Pilihan tidak valid.")
    }
}

// Main — Entry Point
fn main() {
    let system  = Arc::new(Mutex::new(MonitoringSystem::new()));
    let sys_web = Arc::clone(&system);
    let mut sim_idx = 0u32;

    thread::spawn(move || {
        let server = Server::http("127.0.0.1:7878").expect("Server gagal");
        println!("  [WEB] http://127.0.0.1:7878");
        for req in server.incoming_requests() {
            let sys  = sys_web.lock().unwrap();
            let html = render_dashboard(&sys);
            drop(sys);
            let _ = req.respond(Response::from_string(html)
                .with_header(Header::from_bytes("Content-Type","text/html; charset=utf-8").unwrap()));
        }
    });

    thread::sleep(std::time::Duration::from_millis(500));

    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd").args(["/C","start","http://127.0.0.1:7878"]).spawn().ok();
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg("http://127.0.0.1:7878").spawn().ok();
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg("http://127.0.0.1:7878").spawn().ok();

    println!("\n  ELISA — http://127.0.0.1:7878");
    println!("  AMAN <{:.0}mA | WASPADA {:.0}-{:.0}mA | BAHAYA >{:.0}mA\n",
        LEAK_WARN_MA, LEAK_WARN_MA, LEAK_DANGER_MA, LEAK_DANGER_MA);

    loop {
        println!("  [1] Input manual  [2] Simulasi  [3] Injeksi bocor  [4] Keluar");
        let pilih = baca_usize("  > ");

        match pilih {
            1 => {
                let mut sys = system.lock().unwrap();
                input_manual(&mut sys);
                sys.update_all();
                println!("  [OK] Dashboard diperbarui.\n");
            }
            2 => loop {
                {
                    let mut sys = system.lock().unwrap();
                    input_simulasi(&mut sys, sim_idx);
                    sim_idx += 1;
                    sys.update_all();
                }
                println!("  [OK] Simulasi #{} → dashboard diperbarui.", sim_idx);
                if !baca_yn("  Lanjut? [y/n]: ") {
                    let mut sys = system.lock().unwrap();
                                                                    for s in sys.sensors.iter_mut() {
                        s.voltage           = VOLTAGE_NOMINAL;
                        s.current_fasa_ma   = 0.0;
                        s.current_netral_ma = 0.0;
                        s.leak_ma           = 0.0;
                        s.is_valid          = true;
                        s.history_fasa.clear();
                        s.actuator.led_hijau  = true;
                        s.actuator.led_kuning = false;
                        s.actuator.led_merah  = false;
                        s.actuator.buzzer     = false;
                    }
                    println!("  [OK] Semua nilai direset. Sensor standby (AMAN).");
                    break;
                }
            },
            3 => {
                {
                    let mut sys = system.lock().unwrap();
                    let perlu_baseline = sys.sensors.iter().any(|s| s.current_fasa_ma <= 0.0);
                    if perlu_baseline {
                        println!("  [INFO] Mengisi baseline (semua cabang AMAN)...");
                        for i in 0..3 {
                            input_simulasi(&mut sys, i);
                                for s in sys.sensors.iter_mut() {
                                s.current_netral_ma = (s.current_fasa_ma - 1.0).max(0.0);
                            }
                            sys.update_all();
                        }
                        sim_idx = 3;
                    }
                    inject_kebocoran(&mut sys);
                    sys.update_all();
                }
                println!("  [OK] Dashboard diperbarui.\n");
            }
            4 => { println!("\n  Selesai.\n"); break; }
            _ => println!("  [!] Masukkan 1-4.")
        }
        println!();
    }
}