# AI Usage Widget

> Windows 11 Desktop Widget สำหรับติดตามปริมาณการใช้ Token, Quota, Context Window และประวัติการใช้งานจาก **Codex Desktop** และ **Claude Desktop** แบบ Real-time

![Platform](https://img.shields.io/badge/platform-Windows%2011-blue)
![Stack](https://img.shields.io/badge/stack-Tauri%202%20%2B%20React%20%2B%20Rust-orange)
![License](https://img.shields.io/badge/license-MIT-green)

---

## สารบัญ

- [คุณสมบัติหลัก](#คุณสมบัติหลัก)
- [Screenshots](#screenshots)
- [ความต้องการระบบ](#ความต้องการระบบ)
- [การติดตั้ง](#การติดตั้ง)
- [วิธีใช้งานแบบละเอียด](#วิธีใช้งานแบบละเอียด)
- [การตั้งค่า](#การตั้งค่า)
- [แหล่งข้อมูลที่อ่าน](#แหล่งข้อมูลที่อ่าน)
- [สถาปัตยกรรม](#สถาปัตยกรรม)
- [ความปลอดภัยและ Privacy](#ความปลอดภัยและ-privacy)
- [การพัฒนา](#การพัฒนา)
- [Troubleshooting](#troubleshooting)
- [FAQ](#faq)
- [License](#license)

---

## คุณสมบัติหลัก

| ฟีเจอร์                 | รายละเอียด                                                      |
| ----------------------- | --------------------------------------------------------------- |
| 📊 Compact Widget       | หน้าต่าง 340×200px always-on-top แสดง Token usage แบบ real-time |
| 📈 Dashboard            | กราฟประวัติแบบ interactive (Hourly/Daily/Weekly/Monthly)        |
| 🔒 Zero-Token Guarantee | ไม่สร้าง AI inference request ใดๆ ตลอดการทำงาน                  |
| 🇹🇭🇺🇸 Thai/English       | เปลี่ยนภาษาได้ทันทีไม่ต้อง restart                              |
| 🪟 Glass Effect         | Windows 11 Acrylic (blur + transparency)                        |
| 🔔 Smart Notifications  | แจ้งเตือน Windows Toast เมื่อ quota ถึง 75%/90%                 |
| 💾 Backup/Restore       | สำรองข้อมูลพร้อม SHA-256 checksum verification                  |
| 🖱️ Click-through        | เมาส์ผ่าน widget ได้ + Win+Shift+U กลับมาคลิก                   |
| 🎮 Fullscreen Auto-hide | ซ่อนอัตโนมัติเมื่อเปิดแอป fullscreen                            |
| 🔄 Auto Collection      | เก็บข้อมูลทุก 30 วินาที พร้อม exponential backoff               |
| 🎯 Deduplication        | ป้องกันข้อมูลซ้ำด้วย SHA-256 fingerprint + Bloom filter         |
| 📦 Portable Mode        | รันจาก USB/folder ไม่ต้องติดตั้ง                                |

---

## Screenshots

> _TODO: เพิ่ม screenshots เมื่อ build production version สำเร็จ_

---

## ความต้องการระบบ

### สำหรับผู้ใช้งาน (ติดตั้งจาก Installer)

- **OS**: Windows 10 version 1803+ หรือ Windows 11 (64-bit)
- **Runtime**: WebView2 (มากับ Windows 11, Windows 10 อาจต้องติดตั้งเพิ่ม)
- **RAM**: 50MB (ขณะทำงาน)
- **Disk**: ~30MB (ตัวโปรแกรม) + ข้อมูลประมาณ 5-50MB ต่อปี

### สำหรับ Developer (build จาก source)

- **Rust**: 1.70+ (ติดตั้งจาก [rustup.rs](https://rustup.rs/))
- **Node.js**: 18+ (แนะนำ 20 LTS)
- **npm**: 9+
- **Visual Studio Build Tools 2022** พร้อม C++ workload (สำหรับ Rust compilation)

### AI Applications ที่รองรับ

| Application                | เวอร์ชัน    | ข้อมูลที่อ่าน                               |
| -------------------------- | ----------- | ------------------------------------------- |
| Codex Desktop (OpenAI)     | ทุกเวอร์ชัน | Token usage per turn, model, context window |
| Claude Desktop (Anthropic) | ทุกเวอร์ชัน | Quota percentages, daily token count        |

---

## การติดตั้ง

### วิธีที่ 1: NSIS Installer (แนะนำ)

1. ดาวน์โหลด `AI-Usage-Widget_x.x.x_x64-setup.exe` จาก [Releases](../../releases)
2. ดับเบิลคลิกรัน installer
3. ติดตั้งที่ `%LOCALAPPDATA%\Programs\AI Usage Widget\`
4. โปรแกรมจะเริ่มทำงานอัตโนมัติหลังติดตั้ง
5. ข้อมูลเก็บที่ `%APPDATA%\ai-usage-widget\`

### วิธีที่ 2: Portable Mode (ไม่ต้องติดตั้ง)

1. ดาวน์โหลด `AI-Usage-Widget_x.x.x_portable.zip` จาก Releases
2. แตกไฟล์ไปโฟลเดอร์ที่ต้องการ (เช่น USB drive)
3. **สร้างไฟล์ `portable.flag` ว่างๆ** ไว้ข้างๆ `AI Usage Widget.exe`:
   ```
   📁 AI-Usage-Widget/
   ├── AI Usage Widget.exe
   ├── portable.flag          ← สร้างไฟล์นี้ (ไฟล์ว่าง)
   └── data/                  ← โปรแกรมจะสร้างให้เอง
   ```
4. ดับเบิลคลิก `.exe` เพื่อเริ่มใช้งาน
5. ข้อมูลทั้งหมดเก็บใน `./data/` (อยู่ในโฟลเดอร์เดียวกับ exe)

> 💡 **Portable mode** จะไม่เขียน Registry (ไม่มี autostart) — เหมาะสำหรับใช้บน USB หรือเครื่องที่ไม่มีสิทธิ์ admin

### วิธีที่ 3: Build จาก Source

```bash
# 1. Clone repository
git clone https://github.com/<username>/ai-usage-widget.git
cd ai-usage-widget

# 2. ติดตั้ง frontend dependencies
npm install

# 3. Development mode (hot-reload ทั้ง frontend และ backend)
npm run tauri dev

# 4. Production build (สร้าง NSIS installer)
npm run tauri build
# Output: src-tauri/target/release/bundle/nsis/AI-Usage-Widget_0.1.0_x64-setup.exe
```

---

## วิธีใช้งานแบบละเอียด

### เริ่มต้นใช้งาน (First Launch)

1. เปิดโปรแกรม — จะเห็น **Compact Widget** (340×200px) ปรากฏมุมจอ
2. โปรแกรมจะเริ่มเก็บข้อมูลจาก Codex/Claude อัตโนมัติทันที
3. ถ้า provider ไม่พร้อม (ยังไม่ได้ใช้/ไม่มีข้อมูล) จะแสดง "ไม่มีข้อมูล"
4. System Tray icon จะปรากฏที่ taskbar — คลิกขวาเพื่อดูเมนู

### Compact Widget — หน้าจอหลัก

Widget แสดงข้อมูลในรูปแบบ:

```
┌─────────────────────────────────────┐
│  AI Usage Widget    Token: 31,000   │
│─────────────────────────────────────│
│  🟢 Codex Desktop        23,000    │
│     [████████████░░░] Token ทั้งหมด │
│     2 นาทีที่แล้ว                    │
│                                     │
│  🟢 Claude Desktop        8,000    │
│     [██████░░░░░░░░░] Token ทั้งหมด │
│     5 นาทีที่แล้ว                    │
│─────────────────────────────────────│
│  อัปเดตล่าสุด: 10 วินาทีที่แล้ว      │
└─────────────────────────────────────┘
```

**ส่วนประกอบ:**

- **Header**: ชื่อ widget + จำนวน Token รวมวันนี้
- **Provider Row**:
  - 🟢/⚫ จุดแสดงสถานะ (เขียว = ใช้ได้, เทา = ไม่พบข้อมูล)
  - ชื่อ Provider
  - จำนวน Token วันนี้ (format ด้วย comma: 1,234,567)
  - Usage bar (สี: ฟ้า < 75%, เหลือง 75-90%, แดง ≥ 90%)
  - เวลาล่าสุดที่มี activity (relative: "2 นาทีที่แล้ว")
- **Footer**: เวลาอัปเดตล่าสุด

**พฤติกรรม:**

- Auto-refresh ทุก 10 วินาที
- ถ้า Provider ไม่มีข้อมูล แสดง "ไม่มีข้อมูล" (ไม่แสดงเลข 0)
- Glass effect (blur + semi-transparent) ตาม Windows 11 design

---

### System Tray — เมนูคลิกขวา

คลิกขวาที่ไอคอน 🔲 ใน System Tray (มุมขวาล่าง):

| เมนู                 | การทำงาน                  | หมายเหตุ                           |
| -------------------- | ------------------------- | ---------------------------------- |
| **แสดง Widget**      | แสดง + focus widget       | ใช้เมื่อ widget หายไป              |
| **แดชบอร์ด**         | เปิด/ปิด Dashboard window | toggle                             |
| **เก็บข้อมูลตอนนี้** | Trigger collection ทันที  | ไม่ต้องรอ 30 วินาที                |
| **ภาษา**             | สลับ Thai ↔ English       | เปลี่ยนทันที ทุก UI element        |
| **ตั้งค่า**          | เปิดหน้า Settings         | ดูหัวข้อ [การตั้งค่า](#การตั้งค่า) |
| **ออก**              | ปิดโปรแกรมทั้งหมด         | widget + tray หายไป                |

---

### Dashboard — ดูประวัติการใช้งาน

เปิดจาก: System Tray → **แดชบอร์ด** (หรือดับเบิลคลิกที่ widget)

#### ส่วนควบคุม

1. **เลือกช่วงเวลา** (ปุ่มด้านบน):
   - `วัน` — 24 ชั่วโมงล่าสุด
   - `สัปดาห์` — 7 วันล่าสุด (default)
   - `เดือน` — 30 วันล่าสุด
   - `กำหนดเอง` — เลือก start/end date

2. **เลือกความละเอียด** (dropdown):
   - `รายชั่วโมง` — แต่ละจุดบนกราฟ = 1 ชั่วโมง
   - `รายวัน` — แต่ละจุดบนกราฟ = 1 วัน
   - `รายสัปดาห์` — แต่ละจุดบนกราฟ = 1 สัปดาห์
   - `รายเดือน` — แต่ละจุดบนกราฟ = 1 เดือน

#### กราฟ Usage History

- **เส้นฟ้า**: Input Tokens (token ที่ส่งเข้าไป)
- **เส้นเขียว**: Output Tokens (token ที่ AI ตอบกลับ)
- **เส้นเหลือง**: Total Tokens (รวมทั้งหมด)
- Hover เพื่อดูค่าแต่ละจุด (แสดงเวลา Asia/Bangkok)

#### Provider Breakdown (มุมซ้ายล่าง)

แสดงรายละเอียดการใช้งานแยกตาม Provider + Model:

- Input / Output / Total tokens ในช่วงเวลาที่เลือก
- แยกตาม model (เช่น gpt-4o, o1-mini, claude-sonnet-4)

#### Token Type Summary (มุมขวาล่าง)

Bar chart แสดงสัดส่วน:

- Input Tokens (ฟ้า)
- Output Tokens (เขียว)
- Reasoning Tokens (ม่วง)
- Cached Input (ฟ้าอ่อน)

---

### Click-through Mode — เมาส์ผ่าน Widget

**วิธีเปิด:**

1. ไป Settings → เปิด "คลิกทะลุ"
2. Widget จะโปร่ง — เมาส์คลิกผ่านไปหน้าต่างด้านล่างได้

**วิธีปิด (กลับมาคลิก widget):**

- กด **`Win + Shift + U`** (shortcut เดียว จำง่าย)
- Widget จะ focus กลับมา + ปิด click-through อัตโนมัติ

> 💡 เหมาะสำหรับ: เปิด widget ทับ IDE แต่ยังคลิกโค้ดได้ปกติ

---

### Fullscreen Auto-hide

- **ตรวจจับทุก 1 วินาที** ว่ามีแอป fullscreen อยู่ไหม
- เมื่อพบ (เช่น เปิดเกม, ดูวิดีโอ fullscreen): **Widget ซ่อนอัตโนมัติ**
- เมื่อออกจาก fullscreen: **Widget กลับมาเอง**
- ไม่ต้องตั้งค่าอะไร — ทำงานอัตโนมัติ

---

### Notifications — การแจ้งเตือน

Widget จะแจ้งเตือนผ่าน **Windows Toast Notification** เมื่อ:

| ระดับ       | เงื่อนไข    | ข้อความ                            |
| ----------- | ----------- | ---------------------------------- |
| ⚠️ Warning  | Quota ≥ 75% | "คำเตือนการใช้งาน: Claude ที่ 75%" |
| 🚨 Critical | Quota ≥ 90% | "การใช้งานวิกฤต: Claude ที่ 92%"   |

**กฎ Cooldown:**

- แจ้งเตือนซ้ำระดับเดียวกัน provider เดียวกัน ไม่เกิน 1 ครั้ง/ชั่วโมง
- Provider คนละตัว แจ้งแยกกัน (Codex ไม่กระทบ Claude)

---

## การตั้งค่า

เปิดจาก: System Tray → **ตั้งค่า**

### รอบเก็บข้อมูล (Collection Interval)

- **ค่าที่ตั้งได้**: 10 – 3,600 วินาที (slider)
- **Default**: 30 วินาที
- **Adaptive**: ถ้ามี > 50 events ใหม่ ระบบจะย่อรอบเป็น 15 วินาที
- **Backoff**: ถ้าเก็บข้อมูลผิดพลาดติดต่อกัน จะเพิ่มรอบ: 30s → 60s → 120s → 240s → 300s (max)
- เมื่อกลับมาปกติ จะ reset กลับเป็น 30s

### ภาษา (Language)

- **Thai** (default) — ทุก UI text เป็นภาษาไทย
- **English** — ทุก UI text เป็นภาษาอังกฤษ
- เปลี่ยนได้ทันที ไม่ต้อง restart
- System Tray menu เปลี่ยนตาม

### แจ้งเตือน (Notifications)

- **Warning threshold**: default 75% (ปรับได้ 1-100%)
- **Critical threshold**: default 90% (ปรับได้ 1-100%)
- Toast notification จาก Windows
- Cooldown: 1 ชั่วโมง ต่อ provider ต่อ threshold level

### เริ่มอัตโนมัติ (Autostart)

- **เปิด**: เพิ่ม Registry key ใน `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`
- **ปิด**: ลบ Registry key
- ⚠️ ไม่ทำงานใน Portable mode (ไม่เขียน Registry)

### อยู่บนสุดเสมอ (Always on Top)

- Widget อยู่เหนือหน้าต่างอื่นเสมอ
- ปิดได้ถ้าต้องการให้ window อื่นบังได้

### คลิกทะลุ (Click-through)

- เมาส์ผ่าน widget ไปคลิกหน้าต่างด้านล่าง
- กด `Win+Shift+U` เพื่อปิดและกลับมาคลิก widget

### สำรองข้อมูล (Backup)

1. พิมพ์ path ที่ต้องการบันทึก (เช่น `C:\backup\usage.db`)
2. กดปุ่ม "สำรองข้อมูล"
3. ระบบจะ:
   - Export SQLite database (VACUUM INTO)
   - คำนวณ SHA-256 checksum
   - บันทึก record ใน backup_history

### กู้คืนข้อมูล (Restore)

1. พิมพ์ path ของ backup file
2. กดปุ่ม "กู้คืนข้อมูล"
3. ระบบจะ:
   - ตรวจสอบ SHA-256 checksum ก่อน (ถ้าไม่ตรง ปฏิเสธ)
   - สำรอง database ปัจจุบันไว้ก่อน
   - แทนที่ด้วย backup file

### Data Retention

- **Default**: เก็บข้อมูล 365 วัน
- เกิน 365 วัน จะถูกลบอัตโนมัติ (prune_expired)
- Configurable ผ่าน config.json

---

## แหล่งข้อมูลที่อ่าน

Widget อ่านข้อมูลจาก **local files เท่านั้น** — ไม่มี network request ไปยัง AI API:

### Codex Desktop

| รายการ             | รายละเอียด                                                                                              |
| ------------------ | ------------------------------------------------------------------------------------------------------- |
| **Path**           | `%USERPROFILE%\.codex\sessions\YYYY\MM\DD\*.jsonl`                                                      |
| **Format**         | JSON Lines (1 JSON object ต่อบรรทัด)                                                                    |
| **Events ที่อ่าน** | `session_meta` (model name), `token_count` (input/output/total/reasoning/cached tokens, context window) |
| **SQLite**         | `%USERPROFILE%\.codex\state_5.sqlite` — ตาราง `threads` (tokens_used, model, created_at)                |
| **วิธีอ่าน**       | Incremental (จำ byte offset ของแต่ละไฟล์ — อ่านเฉพาะส่วนใหม่)                                           |
| **Lock handling**  | ถ้า SQLite ถูก lock → retry 3 ครั้ง (100ms, 200ms, 400ms) → fall back เป็น JSONL only                   |

### Claude Desktop

| รายการ                      | รายละเอียด                                                                |
| --------------------------- | ------------------------------------------------------------------------- |
| **Path**                    | `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\` |
| **plan-usage-history.json** | Version 2 schema: timestamp (ms), org_id, fast_hours%, standard%, excess% |
| **buddy-tokens.json**       | `tokens-today.date` + `tokens-today.tokens` (daily token count)           |
| **วิธีอ่าน**                | Checkpoint-based (อ่านเฉพาะ samples ที่ timestamp > last checkpoint)      |
| **Windows Store**           | Path เป็น sandboxed — ต้อง access ผ่าน %LOCALAPPDATA%\Packages\ path      |

---

## สถาปัตยกรรม

```
┌─────────────────────────────────────────────────────────────┐
│                    Frontend (React + TypeScript)              │
│  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌───────────┐  │
│  │ Compact  │  │ Dashboard │  │ Settings │  │  Update   │  │
│  │ Widget   │  │  (Charts) │  │  Panel   │  │  Banner   │  │
│  └────┬─────┘  └─────┬─────┘  └────┬─────┘  └─────┬─────┘  │
│       └───────────────┼─────────────┼───────────────┘        │
│                       │ Zustand Store + IPC Bridge            │
└───────────────────────┼──────────────────────────────────────┘
                        │ Tauri IPC (invoke)
┌───────────────────────┼──────────────────────────────────────┐
│                    Backend (Rust)                              │
│  ┌─────────────┐  ┌──────────────┐  ┌───────────────────┐   │
│  │  Collection │→ │ Deduplication │→ │  Reconciliation   │   │
│  │  Scheduler  │  │   (SHA-256    │  │  (merge events)   │   │
│  │  (30s loop) │  │  + Bloom)     │  │                   │   │
│  └──────┬──────┘  └──────────────┘  └─────────┬─────────┘   │
│         │                                       │             │
│  ┌──────┴──────────────────┐           ┌───────┴──────────┐  │
│  │  Provider Adapters      │           │  Storage Layer   │  │
│  │  ┌───────┐  ┌────────┐ │           │  (SQLite + WAL)  │  │
│  │  │ Codex │  │ Claude │ │           │  365 day retain  │  │
│  │  └───────┘  └────────┘ │           └──────────────────┘  │
│  └─────────────────────────┘                                  │
│                                                               │
│  ┌──────────┐ ┌───────────┐ ┌──────────┐ ┌──────────────┐   │
│  │ Network  │ │  Window   │ │  System  │ │ Notification │   │
│  │  Guard   │ │  Manager  │ │   Tray   │ │   Engine     │   │
│  └──────────┘ └───────────┘ └──────────┘ └──────────────┘   │
└───────────────────────────────────────────────────────────────┘
```

### Data Pipeline Flow

```
Codex JSONL + SQLite  ──┐
                        ├→ Raw Events → Dedup → Reconcile → Store → Query → UI
Claude JSON files     ──┘
```

---

## ความปลอดภัยและ Privacy

### สิ่งที่ Widget ไม่ทำ (ห้ามเด็ดขาด)

- ❌ ไม่เก็บ prompt, response, หรือเนื้อหาการสนทนา
- ❌ ไม่ส่ง request ไปยัง AI inference endpoint (openai.com, anthropic.com)
- ❌ ไม่ส่งข้อมูลไปยัง server ภายนอก (ยกเว้น GitHub API ตรวจอัปเดต)
- ❌ ไม่เก็บ raw project paths หรือ session IDs

### สิ่งที่ Widget ทำเพื่อ Privacy

- ✅ Hash project paths ด้วย SHA-256 ก่อนเก็บ (ไม่เห็น path จริง)
- ✅ Hash session IDs ด้วย SHA-256 ก่อนเก็บ
- ✅ Hash org IDs ด้วย SHA-256 ก่อนเก็บ
- ✅ Network Guard: block ทุก outbound request ยกเว้น allowlist
- ✅ CSP headers: `connect-src ipc: https://api.github.com` เท่านั้น
- ✅ HTTP plugin scope: deny openai.com, anthropic.com + all subdomains
- ✅ DevTools ปิดใน production build
- ✅ IPC input validation ทุก command (reject invalid inputs)

### Network Allowlist

| Host                      | วัตถุประสงค์                         |
| ------------------------- | ------------------------------------ |
| `api.github.com`          | ตรวจสอบอัปเดตจาก GitHub Releases     |
| `localhost` / `127.0.0.1` | Tauri dev server (dev mode เท่านั้น) |

### Network Blocklist (block ทั้ง domain + subdomains)

- `openai.com` (api.openai.com, chat.openai.com, etc.)
- `anthropic.com` (api.anthropic.com, console.anthropic.com, etc.)
- `chatgpt.com`
- `claude.ai`

---

## การพัฒนา

### Commands

```bash
# Development (hot-reload)
npm run tauri dev

# Frontend only
npm run dev

# TypeScript type check
npx tsc --noEmit

# Frontend tests (Vitest + React Testing Library)
npm run test

# Rust check (compilation only)
cd src-tauri && cargo check --lib

# Rust tests (property-based + unit)
cd src-tauri && cargo test --lib

# Production build
npm run tauri build
```

### Tech Stack

| Layer     | Technology                   | ทำหน้าที่               |
| --------- | ---------------------------- | ----------------------- |
| Frontend  | React 19 + TypeScript        | UI rendering            |
| State     | Zustand                      | State management        |
| Charts    | Recharts                     | Data visualization      |
| i18n      | i18next + react-i18next      | Thai/English            |
| Styling   | Tailwind CSS 4               | Utility-first CSS       |
| Backend   | Rust                         | Core logic, performance |
| Framework | Tauri 2                      | Desktop app framework   |
| Database  | SQLite (WAL mode)            | Local persistence       |
| Testing   | Vitest, proptest, fast-check | Property-based + unit   |

### โครงสร้างไฟล์

```
ai-usage-widget/
├── src/                          # React frontend
│   ├── components/
│   │   ├── CompactWidget.tsx     # Main 340×200px widget
│   │   ├── Dashboard.tsx         # Expanded charts view
│   │   ├── Settings.tsx          # Settings panel
│   │   ├── UpdateBanner.tsx      # Update notification
│   │   ├── ProviderMeter.tsx     # Usage bar component
│   │   ├── StatusDot.tsx         # Availability dot
│   │   └── LoadingSkeleton.tsx   # Loading placeholder
│   ├── store/index.ts            # Zustand store + actions
│   ├── lib/
│   │   ├── ipc.ts               # Tauri IPC bridge (type-safe)
│   │   └── format.ts            # Time/number formatters
│   ├── i18n/
│   │   ├── index.ts             # i18next config
│   │   ├── th.json              # Thai translations
│   │   └── en.json              # English translations
│   └── tests/
│       └── components.test.tsx   # Frontend component tests
├── src-tauri/                    # Rust backend
│   ├── src/
│   │   ├── lib.rs               # App startup + module registry
│   │   ├── main.rs              # Entry point
│   │   ├── commands.rs          # 9 IPC command handlers
│   │   ├── config.rs            # Configuration structs
│   │   ├── types.rs             # Core data types
│   │   ├── error.rs             # Error types
│   │   ├── providers/
│   │   │   ├── codex.rs         # Codex Desktop adapter
│   │   │   └── claude.rs        # Claude Desktop adapter
│   │   ├── dedup.rs             # SHA-256 deduplication + Bloom
│   │   ├── reconcile.rs         # Event merging engine
│   │   ├── storage.rs           # SQLite CRUD + backup/restore
│   │   ├── scheduler.rs         # Collection loop + backoff
│   │   ├── network.rs           # Network Guard (Zero-Token)
│   │   ├── window.rs            # Window manager + click-through
│   │   ├── tray.rs              # System tray + autostart
│   │   ├── notify.rs            # Notification engine
│   │   ├── privacy.rs           # SHA-256 hashing utilities
│   │   ├── validation.rs        # IPC input validators
│   │   ├── query_types.rs       # Query/response types
│   │   ├── registry.rs          # Provider registry
│   │   └── integration_tests.rs # Full pipeline integration tests
│   ├── Cargo.toml               # Rust dependencies
│   └── tauri.conf.json          # Tauri configuration
├── package.json                  # Frontend dependencies
├── vite.config.ts               # Vite build config
├── tsconfig.json                # TypeScript config
└── .kiro/specs/                 # Specification documents
    └── ai-usage-widget/
        ├── requirements.md
        ├── design.md
        └── tasks.md
```

---

## Troubleshooting

### Widget ไม่แสดง Codex data

1. ตรวจสอบว่ามีโฟลเดอร์ `%USERPROFILE%\.codex\sessions\` อยู่
2. ต้องใช้ Codex Desktop อย่างน้อย 1 ครั้ง (สร้าง session) ก่อน
3. ดู System Tray → เก็บข้อมูลตอนนี้ เพื่อ trigger manual

### Widget ไม่แสดง Claude data

1. ตรวจสอบว่า Claude Desktop ติดตั้งจาก Microsoft Store
2. Path ต้องเป็น: `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\`
3. ต้องมีไฟล์ `plan-usage-history.json` อยู่ใน path ดังกล่าว
4. ถ้าติดตั้ง Claude แบบ standalone (ไม่ใช่ Store) — path จะต่างไป ต้องแก้ config

### Widget หายจากหน้าจอ

- อาจอยู่ในโหมด Fullscreen Auto-hide → ลอง minimize fullscreen app
- หรือกด `Win+Shift+U` เพื่อ focus widget
- หรือ System Tray → แสดง Widget

### Build ไม่ผ่าน (path has spaces)

- Rust windres มีปัญหากับ path ที่มี spaces
- แก้โดย set `CARGO_TARGET_DIR=D:\cargo_target` ก่อน build
- หรือ clone project ไปยัง path ที่ไม่มี spaces

### WebView2 ไม่มี (Windows 10)

- ดาวน์โหลด WebView2 Runtime จาก [Microsoft](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
- Windows 11 มีอยู่แล้วไม่ต้องทำอะไร

---

## FAQ

**Q: Widget กิน Token ของ AI ไหม?**

> ไม่ครับ Zero-Token Guarantee — widget อ่านจาก local files เท่านั้น ไม่สร้าง API request ใดๆ

**Q: Widget ส่งข้อมูลของฉันไปไหน?**

> ไม่ส่งไปไหนเลย ข้อมูลเก็บอยู่ใน local SQLite database เท่านั้น Network request มีแค่ตรวจอัปเดตจาก GitHub

**Q: เพิ่ม Provider ใหม่ได้ไหม?**

> ได้ — implement `ProviderAdapter` trait ใน Rust และ register ใน `ProviderRegistry` ระบบออกแบบเป็น plugin pattern

**Q: ข้อมูลเก็บนานแค่ไหน?**

> Default 365 วัน หลังจากนั้นลบอัตโนมัติ (configurable)

**Q: Backup ไปเครื่องใหม่ได้ไหม?**

> ได้ — ใช้ Backup/Restore ใน Settings ระบบจะ verify checksum ก่อน restore

**Q: รองรับ macOS/Linux ไหม?**

> ปัจจุบันรองรับ Windows เท่านั้น (ใช้ Win32 API สำหรับ click-through, fullscreen detection, registry autostart)

**Q: จะรู้ได้ไงว่า widget ไม่ได้แอบส่ง token?**

> 1. Network Guard block ทุก AI endpoint ที่ code level
> 2. Tauri HTTP plugin scope: deny openai.com + anthropic.com
> 3. CSP headers: connect-src อนุญาตเฉพาะ github.com
> 4. ไม่มี AI SDK ใน dependencies (ตรวจได้จาก Cargo.toml + package.json)

---

## Keyboard Shortcuts

| Shortcut          | การทำงาน                         |
| ----------------- | -------------------------------- |
| `Win + Shift + U` | ปิด Click-through + Focus widget |

---

## Config File

ไฟล์ config อยู่ที่:

- **Installed mode**: `%APPDATA%\ai-usage-widget\config.json`
- **Portable mode**: `<exe_dir>\data\config.json`

ตัวอย่าง:

```json
{
  "collection_interval_secs": 30,
  "retention_days": 365,
  "locale": "th",
  "db_path": "C:\\Users\\<user>\\AppData\\Roaming\\ai-usage-widget\\usage.db",
  "codex": {
    "enabled": true,
    "sessions_dir": "C:\\Users\\<user>\\.codex\\sessions",
    "state_db_path": "C:\\Users\\<user>\\.codex\\state_5.sqlite"
  },
  "claude": {
    "enabled": true,
    "data_dir": "C:\\Users\\<user>\\AppData\\Local\\Packages\\Claude_pzs8sxrjxfjjc\\LocalCache\\Roaming\\Claude"
  },
  "window": {
    "width": 340,
    "height": 200,
    "always_on_top": true,
    "click_through": false
  }
}
```

---

## License

MIT — ใช้งาน แก้ไข แจกจ่ายได้อย่างอิสระ
