# Picasso

[English](README.md) | 繁體中文

![Picasso](images/banner.jpg)

Picasso 是適用於 **Windows、macOS 與 Linux** 的免費開源相片管理、檢視與編輯工具。您可以直接管理現有資料夾中的相片，依日期或地點瀏覽，並在不變更原始檔案的情況下編輯相片。

Picasso 以 Picasa 為靈感，使用 Rust 開發。相片資料庫與編輯紀錄儲存在本機，不需要雲端帳號，也不會上傳相片或影片。

[下載](https://github.com/kuopinghsu/picasso/releases/latest) · [使用手冊](docs/USER_GUIDE_zh-TW.adoc) · [版本紀錄](CHANGELOG.md)

![Photo library](images/picasso.png)

## 目錄

- [功能](#功能)

- [安裝](#安裝)

- [開始使用](#開始使用)

- [設定與資料](#設定與資料)

- [隱私與限制](#隱私與限制)

- [文件](#文件)

- [開發](#開發)

- [版本發佈](#版本發佈)

- [參與貢獻](#參與貢獻)

- [授權](#授權)

## 功能

### 整理與尋找相片

- 透過依日期分組的縮圖、資料夾、相簿、說明文字與星號標記管理相片。

- 依檔名、資料夾路徑、說明文字、相機、鏡頭或相簿名稱搜尋，也能組合日期與關鍵字，例如 `2019-05 beach`。

- 使用 `type:raw`、`gps:yes` 與 `is:edited` 等搜尋篩選器，以 `-` 排除字詞，並重用最近的搜尋。

- 依日期或中繼資料樣式重新命名相片與影片。套用前可預覽名稱，重新啟動 Picasso 後也能復原重新命名。

- 在背景掃描資料夾並監控變更，也能匯入 Picasa 中繼資料檔案的星號、說明文字與旋轉設定。

- 透過內容檢查與 SHA-256 驗證尋找重複檔案，檢視結果後再將不需要的副本移至系統垃圾桶。

- 查看日期、相機、鏡頭、焦距與具有位置資訊的相片比例等統計。

### 檢視、編輯與匯出

- 使用縮放、平移、底片列、EXIF 詳細資訊、直方圖與幻燈片功能檢視相片；安裝 FFmpeg 後可播放有聲影片。

- 調整光線、色調曲線、白平衡、色彩、細節與效果，並使用預設集、裁切、透視校正、角點變形及線性或放射狀遮罩。

- 可獨立篩選已編輯相片，並在空白畫面直接清除搜尋或篩選，不影響排序方式。

- 保留編輯歷史，支援復原、重做與前後比較。可將設定複製到其他相片，但不包含裁切與變形設定。

- 將編輯結果個別或批次匯出為 JPEG、PNG 或 TIFF 副本，並選擇輸出尺寸與 JPEG 品質。

![Photo editor](images/editor.png)

可將編輯後的相片複製到剪貼簿、複製檔案路徑，或以預設應用程式開啟檔案。投影片支援隨機播放與淡入淡出。

### 探索地點與 GPS 軌跡

- 在 **Places（地點）** 中使用 OpenFreeMap、OpenStreetMap、CARTO 或 Esri 地圖顯示具有 GPS 座標的相片。可手動放置相片，或依拍攝時間與 GPS 軌跡比對。「地點」也支援搜尋與星號篩選，點選軌跡可找出沿途拍攝的相片。

- 顯示 GPX、KML 軌跡與 Polarsteps JSON 位置匯出資料，編輯軌跡點、移除異常點、簡化路徑，再匯出新的 GPX 或 KML 副本。

- 使用 **Travel Map（旅行地圖）** 查看相片拍攝所在的國家與支援的地區。地球與平面地圖採用內建 Natural Earth 邊界資料，可離線使用。

![Places](images/places.png)

![GPS track editor](images/trackedit.png)

![Travel Map](images/travelmap.png)

![Library statistics](images/statistics.png)

### 語言與外觀

介面支援英文、繁體中文、簡體中文、日文、韓文、德文、西班牙文、法文、義大利文與巴西葡萄牙文。可在 **Settings → General（設定 → 一般）** 選擇語言，以及淺色、深色或系統主題。

## 安裝

從 [GitHub Releases](https://github.com/kuopinghsu/picasso/releases/latest) 下載套件。Intel 或 AMD 系統請選 x64，64 位元 Arm 系統請選 Arm64。發佈檔案也包含英文與繁體中文 PDF 使用手冊，以及 SHA-256 校驗碼檔案。

### Windows

支援 x64 與 Arm64 的 Windows 10、11。下載對應的 `.exe` 後執行，或解壓縮 `.zip` 套件，不需要安裝程式。

### macOS

支援 Apple silicon 與 Intel Mac，需 macOS 11 或更新版本。開啟 `.dmg`，將 **Picasso** 拖曳至 **Applications（應用程式）**。也可使用 `.zip` 內的應用程式套件。

請查看版本說明中的簽署與公證狀態。若未經公證的版本遭到阻擋，先嘗試開啟一次，再到 **系統設定 → 隱私權與安全性** 選擇 **強制打開**。

### Linux

- **AppImage：** 提供 x64 與 Arm64 版本，適用於 Ubuntu 20.04、RHEL 8、Debian 10 及更新版本。執行 `chmod +x <filename>.AppImage` 後開啟檔案。若沒有 FUSE，請加入 `--appimage-extract-and-run` 啟動。

- **Debian 套件：** 適用於 Ubuntu 22.04 及更新版本，使用 `sudo apt install ./<filename>.deb` 安裝。

- **TAR 壓縮檔：** 在 Ubuntu 22.04 建置，包含執行檔、桌面項目與圖示，可手動安裝。

檔案對話框使用 XDG desktop portal，通常由桌面環境提供。AppImage 內含應用程式所需的函式庫，但仍需系統提供 glibc 與顯示驅動程式。

### 選用的影片支援

影片播放與縮圖需要 FFmpeg 和 ffprobe，相片功能不受影響。請依系統使用對應指令安裝 FFmpeg：

- Windows：`winget install Gyan.FFmpeg`

- macOS：`brew install ffmpeg`

- Debian 或 Ubuntu：`sudo apt install ffmpeg`

Picasso 會搜尋 `PATH`、應用程式資料夾、常見的 WinGet 與 Homebrew 路徑，以及 `PICASSO_FFMPEG` 指定的資料夾或 FFmpeg 執行檔。可攜式使用時，請將兩個執行檔放在應用程式旁，或其 `ffmpeg` 子資料夾內。

## 開始使用

1. 啟動 Picasso。首次啟動時，若系統有「圖片」資料夾，程式會自動加入並在背景掃描，逐步顯示相片。
2. 使用 **+ Import（匯入）** 或將資料夾拖曳至視窗以加入其他資料夾。Picasso 直接索引原位置的檔案，不會將檔案搬入專用資料夾。
3. 選取相片後按 **Enter** 檢視、**E** 編輯，或 **S** 標記星號。可利用相簿與說明文字整理相片。
4. 選擇 **Export…（匯出）** 儲存編輯後的副本。匯出前，編輯紀錄保留在資料庫內，原始檔案不變。
5. 按 **F1** 或 **?** 開啟說明與快捷鍵，按 **F5** 重新掃描資料夾。

### 命令列用法

可將相片資料夾路徑傳給執行檔，或使用地理標記指令比對相片與軌跡。下列範例假設 `picasso` 已加入 `PATH`，否則請使用完整路徑。

```sh
picasso ~/Pictures ~/Photos
picasso --version
picasso geotag track.gpx ~/Pictures/Trip --offset +01:00 --dry-run
picasso geotag --help
```

地理標記範例只預覽比對結果，不寫入變更。移除預覽選項後，位置會儲存到資料庫及相片旁的 `.picasso-geotags.json` 檔案。時差請設定為相機時鐘相對於 UTC 的偏移量。

## 設定與資料

使用齒輪按鈕開啟 **Settings（設定）**，調整語言、主題、監控與排除的資料夾、檔案類型、地圖來源、GPS 軌跡預設值及儲存空間。**Settings → Storage（設定 → 儲存空間）** 顯示實際資料路徑，並提供快取與備份控制。

### 儲存位置

資料庫儲存在 `picasso.db`，偏好設定另存於 `settings.json`。預設資料夾如下：

- **Windows：** 資料庫 `%APPDATA%\Picasso\picasso\data`；快取 `%LOCALAPPDATA%\Picasso\picasso\cache`；設定 `%APPDATA%\Picasso\picasso\config\settings.json`。

- **macOS：** 資料庫 `~/Library/Application Support/app.Picasso.picasso`；快取 `~/Library/Caches/app.Picasso.picasso`；設定 `~/Library/Application Support/app.Picasso.picasso/settings.json`。

- **Linux：** 資料庫 `~/.local/share/picasso`；快取 `~/.cache/picasso`；設定 `~/.config/picasso/settings.json`。

快取包含縮圖、地圖圖磚與重複檔案搜尋雜湊，可從設定清除並重新產生。設定檔中的 CARTO API 金鑰使用目前電腦的資訊加密；將設定搬至其他電腦後，必須重新輸入金鑰。

### 備份與復原

啟動時，若最新的資料庫備份已超過一週，或尚無備份，Picasso 就會建立備份，並在資料庫旁的資料夾保留最新五份。也可在 **Settings → Storage（設定 → 儲存空間）** 手動建立或還原備份。

**資料庫備份不包含相片、影片與設定檔，請另外備份這些檔案。** 資料庫備份包含相簿、星號、說明文字、編輯與歷史、預設集及 GPS 軌跡。

若資料庫無法開啟，Picasso 會顯示復原選項。損壞的資料庫可先保留，再以可讀取的備份或新資料庫取代。忙碌中或無權存取的資料庫不會被取代。復原步驟請參閱[使用手冊](docs/USER_GUIDE_zh-TW.adoc)。

重新命名歷史儲存在相片旁的 `.picasso-renames.json`，請與媒體檔案一起保留，以便復原先前的重新命名。

## 隱私與限制

- **網路存取：** 相片瀏覽與編輯在本機執行。線上地圖會下載圖磚，高度查詢會下載軌跡區域的地形圖磚，因此服務提供者可得知所請求的區域。旅行地圖使用內建邊界資料，可離線使用。

- **更新：** 預設啟用自動更新檢查，每天最多向 GitHub 查詢一次。可在 **Settings → General（設定 → 一般）** 關閉。更新檔案須從發佈頁面手動下載。

- **原始檔案：** 相片編輯、旋轉與日期修正儲存在資料庫，GPS 比對另會寫入附屬檔案。重新命名檔案或將相片移至垃圾桶等明確的檔案操作，會影響磁碟上的檔案。

- **格式：** 支援 JPEG、PNG、GIF、WebP、BMP、TIFF、HEIC/HEIF 與相機 RAW。部分不支援的 RAW 機型會使用相機內嵌的 JPEG 預覽。影片需要 FFmpeg，且無法編輯。

- **編輯範圍：** 不提供 AI 遮罩、修復、多張相片 HDR 合併、全景接合或鏡頭描述檔。HDR 控制只調整單張相片。

- **軌跡匯入：** 資料庫掃描支援 GPX、KML 與 Polarsteps JSON，不索引 KMZ 壓縮檔或區域輪廓。

## 文件

- [英文使用手冊](docs/USER_GUIDE.adoc)與[繁體中文使用手冊](docs/USER_GUIDE_zh-TW.adoc)：完整操作流程、快捷鍵與疑難排解。

- [資料庫](docs/DATABASE.md)：資料庫內容、結構版本與升級。

- [旅程與故事](docs/TRAVEL.md)：旅行相片如何整理成旅程與故事，以及背後的設計決定。

- [測試](docs/TESTING.md)：自動化測試範圍與發佈前的人工檢查。

- [效能](docs/PERFORMANCE.md)：基準測試及外接、網路磁碟的行為。

- [發佈流程](docs/RELEASING.md)與[版本紀錄](CHANGELOG.md)。

## 開發

### 從原始碼建置

安裝 **1.95 或更新版本的 Rust** 與 C 編譯器，SQLite 需要從原始碼建置。Windows 請安裝 Visual Studio Build Tools 的 **使用 C++ 的桌面開發** 與 Windows SDK。macOS 請使用 `xcode-select --install` 安裝命令列工具。

Debian 或 Ubuntu 請先安裝建置相依套件：

```sh
sudo apt install build-essential pkg-config libasound2-dev libxkbcommon-dev libgl1-mesa-dev
```

Fedora 請安裝對應套件：

```sh
sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel libxkbcommon-devel mesa-libGL-devel
```

在儲存庫根目錄執行：

```sh
cargo build --release --locked
cargo run --release --locked -- ~/Pictures
cargo test --release --locked
```

macOS 與 Linux 的執行檔為 `target/release/picasso`，Windows 則為 `target/release/picasso.exe`。Release 建置的掃描、縮圖與編輯效能優於除錯建置。

在 macOS 上，每次建置都以臨時（ad hoc）方式簽署，對鑰匙圈而言都是新的 App，因此每個已儲存的 WordPress 密碼都會詢問兩次，下次建置時也會忘記**永遠允許**。若要讓每次建置都以相同方式簽署，請先建立一次程式碼簽署憑證：在「鑰匙圈存取」中選擇**憑證輔助程式 → 製作憑證…**，名稱設為 `Picasso Development`，**憑證類型**選**程式碼簽署**，然後建立。之後 `make` 與 `make run` 會用它簽署 `target/release/picasso`（可用 `SIGN_IDENTITY=...` 指定其他名稱）；第一次選擇**永遠允許**後，之後的建置就不會再詢問。直接用 `cargo` 建置的版本不會以此方式簽署。

預設使用 OpenGL 繪圖。設定 `PICASSO_RENDERER=wgpu` 可改用 wgpu 繪圖後端。

### 專案結構

- 應用程式介面與操作流程：[src/app/](src/app)。

- 影像處理與編輯設定：[src/edit/](src/edit)。

- 資料庫、掃描與縮圖：[src/db.rs](src/db.rs)、[src/worker.rs](src/worker.rs) 與 [src/thumbs.rs](src/thumbs.rs)。

- GPS 軌跡與地理標記：[src/track.rs](src/track.rs)、[src/track_edit.rs](src/track_edit.rs) 與 [src/geotag.rs](src/geotag.rs)。

- 翻譯：[src/i18n/](src/i18n)；語言選擇邏輯位於 [src/i18n.rs](src/i18n.rs)。

### 檢查與維護

[Makefile](Makefile) 提供建置、測試、基準測試、文件與打包指令。執行 `make help` 查看所有目標，常用指令如下：

- `make check`：檢查應用程式與測試的型別。

- `make lint`：執行 Clippy，顯示警告但不視為錯誤。

- `make test`：執行 Release 模式測試；可用 `make test T=backup` 依名稱篩選。

- `make check-docs`：依原始碼驗證文件連結、指令與設定。

- `make ci`：檢查文件與相依套件、執行測試並檢查基準測試門檻，需要 cargo-deny。

- `make guide` 與 `make guide-pdf`：將兩種語言的使用手冊建置為 HTML 或 PDF，需要 Asciidoctor 或 Asciidoctor PDF，以及 Makefile 說明的字型。

CI 使用原生 x64 與 Arm64 執行環境建置及測試 Windows、Linux 套件，並產生 macOS 通用應用程式。另會檢查 Linux 套件安裝與啟動、AppImage 相容性、文件及相依套件授權。完整檢查請參閱[工作流程](.github/workflows/ci.yml)。

## 版本發佈

例如 `v0.9.5` 的版本標籤會觸發發佈。標籤必須符合 Cargo.toml 的版本，以及 CHANGELOG.md 中已填寫日期的章節。含連字號的標籤會作為預先發佈版本。CI 讀取版本紀錄產生發佈說明，不會修改檔案。

每次發佈包含兩種架構的 Windows 執行檔與 ZIP 套件、Linux TAR、Debian 與 AppImage 套件，以及 macOS 通用 DMG 與 ZIP 套件。另包含兩份 PDF 手冊、RAW 解碼器原始碼與 SHA-256 校驗碼。封裝套件包含第三方授權聲明，單獨的 Windows 執行檔除外。

建立標籤前，請依照[發佈檢查清單](docs/RELEASING.md)操作。已發佈版本保持不變；若發佈失敗或內容有誤，請建立新的修補版本，不要移動標籤或替換已發佈檔案。

## 參與貢獻

歡迎透過 [GitHub Issues](https://github.com/kuopinghsu/picasso/issues) 與 pull request 回報問題、建議功能、改善文件或提供翻譯。回報問題時，請說明重現步驟並附上 **About（關於）** 顯示的版本。提交程式碼變更前，請執行相關測試與文件檢查。

## 授權

Picasso 採用 [MIT 授權](LICENSE)。第三方相依套件各有其授權。可用 `make notices` 產生授權聲明，發佈套件內含產生的聲明檔案。
