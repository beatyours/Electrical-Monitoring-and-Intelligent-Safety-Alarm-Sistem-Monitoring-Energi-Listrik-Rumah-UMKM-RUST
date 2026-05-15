# ⚡ ELISA — Electrical Monitoring and Intelligent Safety Alarm

<div align="center">

![Rust](https://img.shields.io/badge/Rust-1.75+-orange?style=for-the-badge&logo=rust)
![License](https://img.shields.io/badge/License-MIT-blue?style=for-the-badge)
![Status](https://img.shields.io/badge/Status-Active-green?style=for-the-badge)
![ITS](https://img.shields.io/badge/ITS-Teknik%20Instrumentasi-blue?style=for-the-badge)

**Sistem Monitoring Energi Listrik Rumah / UMKM Berbasis Rust**

*ETS Algoritma Pemrograman — Pengembangan Sistem Instrumentasi Pengukuran dan Kontrol*

</div>

---

## 📋 Deskripsi Project

**ELISA** (*Electrical Monitoring and Intelligent Safety Alarm*) adalah sistem monitoring energi listrik berbasis perangkat lunak **Rust** yang dirancang untuk mendeteksi kebocoran arus secara *real-time* pada instalasi listrik rumah tangga dan UMKM.

Sistem ini memantau tiga cabang MCB (**DAPUR**, **KAMAR**, **R.TAMU**) menggunakan prinsip diferensial arus berdasarkan hukum Kirchhoff:

```
I_bocor = I_fasa − I_netral
```

Jika arus bocor melebihi ambang batas, sistem akan mengaktifkan peringatan bertingkat sesuai standar **PUIL 2011**, ditampilkan secara *real-time* melalui *dashboard* web yang dapat diakses dari *browser*.

### 🎯 Fitur Utama

| Fitur | Keterangan |
|-------|-----------|
| 🔍 Deteksi Kebocoran Real-Time | Monitoring 3 cabang MCB secara bersamaan |
| 📊 Dashboard Web | Auto-refresh setiap 3 detik di `http://127.0.0.1:7878` |
| 📈 Moving Average MA-5 | Smoothing noise dari pembacaan sensor ADC |
| 📉 Standar Deviasi | Early warning kestabilan arus |
| ⚡ Akumulasi Energi | Estimasi konsumsi energi (kWh) dengan metode Euler |
| 🚨 Alarm Bertingkat | AMAN / WASPADA / BAHAYA sesuai PUIL 2011 |
| 🔒 Concurrency Aman | `Arc<Mutex<>>` untuk thread-safe state sharing |

---

## 🚦 Threshold Kebocoran (PUIL 2011)

| Status | Rentang I_bocor | LED | Buzzer |
|--------|----------------|-----|--------|
| 🟢 **AMAN** | < 15 mA | Hijau ON | OFF |
| 🟡 **WASPADA** | 15 – 22 mA | Kuning ON | OFF |
| 🔴 **BAHAYA** | > 22 mA | Merah ON | **ON** |

> ⚠️ Nilai 15 mA adalah ambang batas di mana seseorang masih bisa melepaskan konduktor secara mandiri. Di atas 50 mA dapat menyebabkan fibrilasi jantung.

---

## 🏗️ Arsitektur Program

```
ELISA
├── LeakStatus (enum)         → Status kebocoran: Aman, Waspada, Bahaya
├── Actuator (struct)         → Output fisik: LED Hijau/Kuning/Merah + Buzzer
├── Sensor (struct + impl)    → Data & komputasi per cabang MCB
│   ├── validate()            → Validasi range tegangan & arus
│   ├── compute_leak()        → I_bocor = I_fasa − I_netral
│   ├── push_history()        → Buffer geser VecDeque untuk MA-5
│   ├── moving_average()      → Rata-rata 5 pembacaan terakhir
│   ├── std_deviation()       → Standar deviasi sampel (n−1)
│   └── update_actuator()     → Sinkronisasi LED & buzzer
├── Controller (struct + impl)→ Pencatatan kejadian & pesan tindakan
├── MonitoringSystem          → Koordinator seluruh sensor & controller
│   ├── update_all()          → Pipeline: validate→leak→MA→actuator→energy
│   ├── total_power()         → Σ daya semua cabang (W)
│   ├── dominant_branch()     → Cabang dengan beban tertinggi
│   ├── update_energy()       → Akumulasi kWh (Euler forward)
│   └── overall_status()      → Status agregat sistem
├── render_dashboard()        → HTML generator untuk web server
└── main()                    → Web server thread + loop menu terminal
```

---

## 📐 Komputasi Numerik

### 1. Kalkulasi Arus Bocor
```rust
fn compute_leak(&mut self) {
    self.leak_ma = (self.current_fasa_ma - self.current_netral_ma).max(0.0);
}
```

### 2. Moving Average MA-5
```rust
fn moving_average(&self) -> f32 {
    self.history_fasa.iter().sum::<f32>() / self.history_fasa.len() as f32
}
```

### 3. Standar Deviasi Sampel
```
σ = √( Σ(xᵢ − x̄)² / (n − 1) )
```

### 4. Akumulasi Energi (Integrasi Euler)
```
E_total += (P_total / 1000) × Δt   [kWh]
```

---

## 🛠️ Teknologi

- **Bahasa:** Rust 1.75+
- **Web Server:** [`tiny_http`](https://crates.io/crates/tiny-http)
- **Concurrency:** `Arc<Mutex<>>` (standard library)
- **Data Structure:** `VecDeque` untuk buffer MA-5
- **Target Hardware:** ESP32 + Sensor SCT-013 *(integrasi mendatang)*

---

## 📁 Struktur Repository

```
elisa-monitoring/
├── src/
│   └── main.rs              # Source code utama
├── Cargo.toml               # Konfigurasi dependencies Rust
├── Cargo.lock
├── docs/
│   ├── laporan/
│   │   ├── ETS_ALPROG.tex   # File laporan LaTeX
│   │   └── ETS_ALPROG.pdf   # File laporan PDF
│   └── flowchart/
│       └── Flowchart_alprog_drawio.png
├── screenshots/
│   ├── dashboard_aman.png
│   ├── dashboard_waspada.png
│   ├── dashboard_bahaya.png
│   └── terminal_menu.png
└── README.md
```

---

## 🚀 Cara Menjalankan Program

### Prasyarat

Pastikan **Rust** sudah terinstal. Jika belum:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
Verifikasi instalasi:
```bash
rustc --version
cargo --version
```

### Instalasi & Menjalankan

**1. Clone repository**
```bash
git clone https://github.com/<username>/elisa-monitoring.git
cd elisa-monitoring
```

**2. Build project**
```bash
cargo build
```

**3. Jalankan program**
```bash
cargo run
```

**4. Buka dashboard**

Program akan secara otomatis membuka browser. Jika tidak, buka manual:
```
http://127.0.0.1:7878
```

### Dependencies (`Cargo.toml`)

```toml
[package]
name    = "elisa"
version = "0.1.0"
edition = "2021"

[dependencies]
tiny-http = "0.12"
```

---

## 🖥️ Cara Menggunakan Program

Setelah program berjalan, akan muncul menu di terminal:

```
ELISA — http://127.0.0.1:7878
AMAN <15mA | WASPADA 15-22mA | BAHAYA >22mA

[1] Input manual  [2] Simulasi  [3] Injeksi bocor  [4] Keluar
>
```

### Menu [1] — Input Manual
Masukkan nilai tegangan, arus fasa, dan arus netral untuk setiap cabang MCB secara manual melalui terminal.

### Menu [2] — Simulasi Otomatis
Program menghasilkan data deterministik secara otomatis. Cocok untuk demonstrasi Moving Average dan standar deviasi. Tekan `y` untuk melanjutkan iterasi berikutnya, `n` untuk berhenti.

### Menu [3] — Injeksi Kebocoran
Mode demonstrasi untuk mensimulasikan skenario kebocoran:

| Opsi | Aksi |
|------|------|
| 1 | Injeksi ~18 mA → **WASPADA** pada cabang pilihan |
| 2 | Injeksi ~30 mA → **BAHAYA** pada cabang pilihan |
| 3 | Reset cabang tertentu ke **AMAN** |
| 4 | Reset semua cabang ke **AMAN** |

### Menu [4] — Keluar
Menghentikan program.

---

## 📸 Screenshot

### Dashboard — Kondisi AMAN
> *(tambahkan screenshot `screenshots/dashboard_aman.png`)*

### Dashboard — Kondisi WASPADA & BAHAYA
> *(tambahkan screenshot `screenshots/dashboard_bahaya.png`)*

### Terminal — Menu Utama
> *(tambahkan screenshot `screenshots/terminal_menu.png`)*

---

## 👥 Kelompok 4

| Nama | NRP |
|------|-----|
| Devan Ale Satria | 2042251047 |
| Sheilka Janice Leandro | 2042251086 |

**Program Studi:** Teknik Instrumentasi  
**Mata Kuliah:** Algoritma Pemrograman  
**Institusi:** Institut Teknologi Sepuluh Nopember (ITS) Surabaya

---

## 📄 Lisensi

Proyek ini dibuat untuk keperluan akademik ETS Algoritma Pemrograman ITS.

---

<div align="center">
<sub>⚡ ELISA v2.0 — ETS Algoritma Pemrograman — Teknik Instrumentasi ITS Surabaya</sub>
</div>
