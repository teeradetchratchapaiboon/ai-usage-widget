# Requirements Document

## Introduction

AI Usage Widget เป็น Windows 11 Desktop Widget สำหรับติดตามปริมาณการใช้ Token, Quota, Context Window และประวัติการใช้งานจากแอปพลิเคชัน AI (Codex Desktop และ Claude Desktop) โดยรับประกันว่าการเก็บข้อมูลจะไม่สร้าง Model Inference Request ใดๆ (Zero-Token Guarantee)

ระบบใช้ Tauri 2 + React + TypeScript + Rust + SQLite โดยอ่านข้อมูลจาก local files เท่านั้น แสดงผลเป็น compact widget (340×200px) พร้อม expanded dashboard รองรับ Thai/English UI และผสานกับ Windows 11 features

## Glossary

- **Widget**: หน้าต่างขนาดเล็ก (340×200px) ที่แสดง always-on-top บนเดสก์ท็อป Windows 11
- **Dashboard**: หน้าต่างขยายที่แสดงรายละเอียดการใช้งานและกราฟประวัติ
- **Provider_Adapter**: ส่วนประกอบที่ encapsulate วิธีการอ่านข้อมูลจาก AI application แต่ละตัว
- **Collection_Scheduler**: ตัวจัดการรอบการเก็บข้อมูลแบบอัตโนมัติ
- **Deduplication_Engine**: ระบบกรองเหตุการณ์ซ้ำโดยใช้ fingerprint hashing
- **Reconciliation_Engine**: ระบบรวมข้อมูลจากหลายแหล่งสำหรับเหตุการณ์เดียวกัน
- **Storage_Layer**: ชั้นจัดเก็บข้อมูลบน SQLite พร้อม retention policy
- **Network_Guard**: ระบบควบคุม outbound network ที่บล็อก inference endpoints
- **Window_Manager**: ตัวจัดการหน้าต่างรวมถึง always-on-top, click-through, DPI scaling
- **Zero_Token_Guarantee**: หลักประกันว่าระบบจะไม่สร้าง Model Inference Request ใดๆ
- **Fingerprint**: SHA-256 hash ที่คำนวณจากเนื้อหาเหตุการณ์เพื่อตรวจจับข้อมูลซ้ำ
- **JSONL**: JSON Lines format ที่ Codex Desktop ใช้บันทึก session logs
- **IPC**: Inter-Process Communication ระหว่าง frontend (WebView) กับ backend (Rust)

## Requirements

### Requirement 1: Provider Adapter Data Collection

**User Story:** As a user, I want the widget to automatically collect usage data from my AI applications, so that I can monitor my token consumption without manual effort.

#### Acceptance Criteria

1. THE Collection_Scheduler SHALL trigger data collection from all registered Provider_Adapters at a configurable interval (default 30 seconds)
2. WHEN a Provider_Adapter's data source is unavailable, THE Widget SHALL mark that provider as "Not available" and continue collecting from other providers
3. WHEN the Codex Provider_Adapter collects data, THE Provider_Adapter SHALL read JSONL session logs incrementally from the last known byte offset
4. WHEN a JSONL file size is smaller than the stored byte offset, THE Provider_Adapter SHALL reset the offset to zero and re-read the entire file
5. WHEN the Codex Provider_Adapter encounters a malformed JSONL line, THE Provider_Adapter SHALL skip that line, log a warning, and continue processing remaining lines
6. WHEN the Claude Provider_Adapter collects data, THE Provider_Adapter SHALL read plan-usage-history.json and buddy-tokens.json from the Windows Store sandboxed directory
7. THE Provider_Adapter SHALL normalize all collected data into RawUsageEvent structures regardless of source format

### Requirement 2: Deduplication

**User Story:** As a user, I want the system to prevent duplicate data from being stored, so that my usage statistics are accurate.

#### Acceptance Criteria

1. THE Deduplication_Engine SHALL compute a deterministic SHA-256 fingerprint for each RawUsageEvent based on provider_id, timestamp, event_type, model, and token values
2. WHEN a RawUsageEvent has the same fingerprint as a previously stored event, THE Deduplication_Engine SHALL discard it
3. WHEN a RawUsageEvent has a fingerprint not found in storage, THE Deduplication_Engine SHALL pass it through for reconciliation
4. THE Deduplication_Engine SHALL use a Bloom filter for fast in-memory rejection followed by definitive SQLite verification for positives
5. WHEN the same event is computed with the same input values at different times, THE Deduplication_Engine SHALL produce identical fingerprints

### Requirement 3: Reconciliation

**User Story:** As a user, I want data from multiple sources about the same event to be merged intelligently, so that I get the most complete picture of my usage.

#### Acceptance Criteria

1. THE Reconciliation_Engine SHALL group deduplicated events by provider_id, timestamp (within 1-second window), and model
2. WHEN multiple events in a group have overlapping fields, THE Reconciliation_Engine SHALL prefer the event with more non-None token fields
3. WHEN events in a group have complementary fields (one has quota data, another has token data), THE Reconciliation_Engine SHALL merge them into a single ReconciledEvent
4. THE Reconciliation_Engine SHALL record source_count indicating how many raw events contributed to each reconciled event
5. THE Reconciliation_Engine SHALL preserve all non-None field values from any source in the reconciled output (no information loss)

### Requirement 4: Data Storage

**User Story:** As a user, I want my usage history stored locally with automatic cleanup, so that I can view trends without worrying about disk space.

#### Acceptance Criteria

1. THE Storage_Layer SHALL persist all ReconciledEvents in a local SQLite database using WAL mode
2. THE Storage_Layer SHALL store all timestamps in UTC (ISO 8601 format)
3. THE Storage_Layer SHALL enforce a configurable retention period (default 365 days) by pruning expired records
4. WHEN querying usage history, THE Storage_Layer SHALL support aggregation at Hourly, Daily, Weekly, and Monthly granularity
5. WHEN a provider has no data available, THE Widget SHALL display "Not available" and SHALL NOT display zero values
6. THE Storage_Layer SHALL support backup to a specified file path with SHA-256 checksum verification
7. THE Storage_Layer SHALL support restore from a backup file after verifying its checksum

### Requirement 5: Zero-Token Guarantee

**User Story:** As a user, I want absolute assurance that the widget never generates AI model inference requests, so that my usage tracking does not consume any of my quota.

#### Acceptance Criteria

1. THE Network_Guard SHALL block all outbound HTTP requests to known model inference endpoints (api.openai.com, api.anthropic.com, and their subdomains)
2. THE Network_Guard SHALL allow outbound requests only to explicitly allowlisted hosts (api.github.com for update checks)
3. THE Provider_Adapter SHALL collect data exclusively by reading local files (filesystem operations) and SHALL NOT make any network requests
4. THE Widget SHALL NOT include any AI SDK libraries (openai, anthropic, etc.) in its runtime dependencies
5. WHEN the Network_Guard encounters a request to a blocked endpoint, THE Network_Guard SHALL deny the request and log the attempt
6. THE Widget SHALL disable the HTTP client capability in the frontend JavaScript context

### Requirement 6: Compact Widget Display

**User Story:** As a user, I want a small always-visible widget showing my current AI usage at a glance, so that I stay aware of my consumption without switching contexts.

#### Acceptance Criteria

1. THE Widget SHALL render in a 340×200 pixel window (logical pixels, DPI-independent)
2. THE Widget SHALL display token usage, quota percentages, and context window utilization for each available provider
3. WHEN a provider is unavailable, THE Widget SHALL display "Not available" text instead of zero values
4. THE Widget SHALL refresh displayed data every 10 seconds
5. THE Widget SHALL apply Windows 11 glass (Mica/Acrylic) visual effect
6. THE Widget SHALL display relative timestamps for last-updated information

### Requirement 7: Expanded Dashboard

**User Story:** As a user, I want an expanded view with charts and detailed history, so that I can analyze my usage patterns over time.

#### Acceptance Criteria

1. WHEN the user clicks the expand action on the compact widget, THE Widget SHALL open the Dashboard window
2. THE Dashboard SHALL display usage history charts with configurable time range (day, week, month, custom)
3. THE Dashboard SHALL support aggregation granularity selection (Hourly, Daily, Weekly, Monthly)
4. THE Dashboard SHALL display per-provider breakdowns with model-level detail
5. THE Dashboard SHALL display total tokens (input, output, reasoning) for the selected period

### Requirement 8: Windows Integration

**User Story:** As a user, I want the widget to integrate seamlessly with Windows 11, so that it feels like a native part of my desktop.

#### Acceptance Criteria

1. THE Widget SHALL display a system tray icon with a context menu for quick actions (show widget, show dashboard, collect now, language switch, settings, quit)
2. THE Widget SHALL support Windows autostart via Registry entry
3. THE Widget SHALL enforce single-instance operation using a named Windows mutex
4. WHEN another instance is launched, THE Widget SHALL activate the existing instance window and exit
5. THE Widget SHALL maintain always-on-top positioning
6. WHEN a fullscreen application is detected in the foreground, THE Widget SHALL auto-hide
7. WHEN the fullscreen application exits, THE Widget SHALL auto-show
8. THE Widget SHALL support click-through mode where mouse events pass through to windows below
9. WHEN Win+Shift+U is pressed, THE Widget SHALL disable click-through mode and bring the widget to focus
10. THE Widget SHALL scale correctly across different DPI settings and multiple monitors

### Requirement 9: Localization

**User Story:** As a Thai user, I want the widget to display in Thai by default with English as an option, so that I can use it comfortably in my preferred language.

#### Acceptance Criteria

1. THE Widget SHALL default to Thai language (locale: "th") on first launch
2. THE Widget SHALL support switching between Thai and English without application restart
3. WHEN the locale is changed, THE Widget SHALL update all visible text including system tray menu items
4. THE Widget SHALL display timestamps in Asia/Bangkok timezone for the UI while storing in UTC internally

### Requirement 10: Notifications

**User Story:** As a user, I want to be alerted when my usage approaches limits, so that I can adjust my behavior before hitting quotas.

#### Acceptance Criteria

1. WHEN a provider's quota usage reaches 75%, THE Notification_Engine SHALL display a warning notification via Windows Toast
2. WHEN a provider's quota usage reaches 90%, THE Notification_Engine SHALL display a critical notification via Windows Toast
3. THE Notification_Engine SHALL NOT repeat the same threshold notification within a 1-hour cooldown period
4. THE Notification_Engine SHALL include the provider name and current percentage in the notification message

### Requirement 11: Privacy and Security

**User Story:** As a user, I want my data to remain private and secure on my local machine, so that no sensitive information is exposed.

#### Acceptance Criteria

1. THE Storage_Layer SHALL hash project paths using SHA-256 before storage (storing project_hash, not raw paths)
2. THE Storage_Layer SHALL hash session IDs using SHA-256 before storage (storing session_hash, not raw IDs)
3. THE Storage_Layer SHALL hash organization IDs using SHA-256 before storage
4. THE Widget SHALL NOT store any prompt content, response content, or conversation text
5. THE Widget SHALL disable Chromium DevTools in production builds
6. THE Widget SHALL validate all IPC inputs at the Rust command boundary before processing
7. IF an IPC command receives invalid input, THEN THE Widget SHALL return a descriptive error and reject the command

### Requirement 12: Distribution and Updates

**User Story:** As a user, I want easy installation options and update notifications, so that I can keep the widget current without complex procedures.

#### Acceptance Criteria

1. THE Widget SHALL be distributable as an NSIS installer targeting %LOCALAPPDATA%\Programs\AIUsageWidget\
2. THE Widget SHALL be distributable as a portable executable that stores data relative to its location
3. WHEN a portable.flag file exists next to the executable, THE Widget SHALL operate in portable mode without Registry modifications
4. THE Widget SHALL check GitHub Releases API for available updates on startup and periodically
5. WHEN an update is available, THE Widget SHALL notify the user with version info and download link
6. THE Widget SHALL NOT auto-install updates (notification and manual download only)

### Requirement 13: Codex Desktop Data Parsing

**User Story:** As a Codex Desktop user, I want the widget to extract detailed token usage from my session logs, so that I can see per-model, per-session usage breakdowns.

#### Acceptance Criteria

1. THE Codex_Provider_Adapter SHALL discover JSONL files in the ~/.codex/sessions/YYYY/MM/DD/ directory structure
2. THE Codex_Provider_Adapter SHALL parse token_count events extracting input_tokens, cached_input_tokens, output_tokens, reasoning_output_tokens, total_tokens, and model_context_window
3. THE Codex_Provider_Adapter SHALL parse session_meta events to identify the model used
4. THE Codex_Provider_Adapter SHALL read the threads table from state_5.sqlite for summary-level data (tokens_used, model, created_at, updated_at)
5. IF state_5.sqlite is locked by Codex, THEN THE Codex_Provider_Adapter SHALL retry with exponential backoff (max 3 attempts) and fall back to JSONL-only data
6. THE Codex_Provider_Adapter SHALL track per-file byte offsets for incremental reading

### Requirement 14: Claude Desktop Data Parsing

**User Story:** As a Claude Desktop user, I want the widget to display my quota usage and daily token count, so that I know how much of my plan I have consumed.

#### Acceptance Criteria

1. THE Claude_Provider_Adapter SHALL parse plan-usage-history.json (version 2 schema) extracting timestamp, fast hours percentage (fh), standard percentage (sd), and excess percentage (xu)
2. THE Claude_Provider_Adapter SHALL parse buddy-tokens.json extracting the daily token count and date
3. THE Claude_Provider_Adapter SHALL read from the Windows Store sandboxed path (%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\)
4. THE Claude_Provider_Adapter SHALL only process samples newer than the last collection checkpoint
5. IF the Claude data directory is inaccessible, THEN THE Claude_Provider_Adapter SHALL mark Claude as unavailable and log the specific path and error

### Requirement 15: Backup and Restore

**User Story:** As a user, I want to backup and restore my usage data, so that I can recover from data loss or migrate to a new machine.

#### Acceptance Criteria

1. WHEN the user triggers a backup, THE Storage_Layer SHALL export the SQLite database to the specified path
2. THE Storage_Layer SHALL compute and store a SHA-256 checksum of the backup file
3. WHEN the user triggers a restore, THE Storage_Layer SHALL verify the backup file checksum before restoring
4. IF the backup file checksum does not match, THEN THE Storage_Layer SHALL reject the restore and notify the user
5. THE Storage_Layer SHALL maintain a backup_history record with timestamp, path, size, and checksum
6. THE Storage_Layer SHALL support rollback to the most recent valid backup

### Requirement 16: Error Resilience

**User Story:** As a user, I want the widget to handle errors gracefully without crashing, so that it remains useful even when some data sources have issues.

#### Acceptance Criteria

1. IF a Provider_Adapter encounters an error during collection, THEN THE Collection_Scheduler SHALL log the error and continue with other providers
2. WHEN consecutive collection errors occur, THE Collection_Scheduler SHALL apply exponential backoff (30s, 60s, 120s, max 300s)
3. WHEN consecutive errors resolve, THE Collection_Scheduler SHALL reset to the default collection interval
4. IF the SQLite database write fails, THEN THE Widget SHALL log the error, notify the user, and continue operating with cached data
5. THE Widget SHALL never crash due to a single provider's data source being malformed or unavailable
