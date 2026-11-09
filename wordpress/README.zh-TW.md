# VoyageLight WordPress 佈景主題與 Picasso Places 外掛

[English](README.md) | 繁體中文

這是一套可選的傳統 WordPress 旅行網站佈景主題，以及一個可選的外掛，把佈景主題的位置、地圖與旅程帶到任何其他佈景主題。Picasso 兩者都不需要：發佈適用於任何佈景主題，Picasso 也不會讀取這個資料夾。

| 路徑 | 內容 |
| --- | --- |
| `voyagelight/` | 佈景主題的原始碼，在這裡編輯 |
| `voyagelight/languages/` | `voyagelight.pot`（待翻譯的文字）與 `zh_TW.po`（繁體中文） |
| `voyagelight/readme.txt` | 佈景主題自己的說明：描述、常見問題、版本紀錄、授權 |
| `picasso-places/` | Picasso Places 外掛的原始碼 |
| `picasso-places/languages/` | `picasso-places.pot` 與 `picasso-places-zh_TW.po` |
| `picasso-places/readme.txt` | 外掛自己的說明：描述、常見問題、隱私、版本紀錄 |
| `tests/` | 位置讀取程式的 PHP 與 JavaScript 測試，以及外掛防護測試 |

## 建置

```sh
make theme
```

這會確認 PHP 檔案中的每段文字在每個 `.po` 都有翻譯，且 `.pot` 與 `.po` 和 PHP 一致（見「語言」），然後在 `wordpress/` 寫出兩者的英文版與繁體中文版：`voyagelight-<version>-en.zip`、`voyagelight-<version>-zh-TW.zip`、`picasso-places-<version>-en.zip` 與 `picasso-places-<version>-zh-TW.zip`。每個版本不論網站語言為何，都以自己的語言顯示；中文版內含編譯成 `.mo` 的翻譯，兩個版本都不含 `.po` 與 `.pot` 檔。佈景主題的版本來自 `voyagelight/style.css`，外掛的版本來自 `picasso-places/picasso-places.php`，且必須與其 `const VERSION` 以及 `readme.txt` 的 Stable tag 一致。以相同的檔案建置，會得到位元組完全相同的 ZIP。`python3 scripts/build-theme.py --check` 只執行檢查；CI 會執行這項檢查，並對每個 PHP 檔案執行 `php -l`，以及下面的測試。

位置讀取程式有 PHP 版（`voyagelight/inc/geo.php`）與編輯器用的 JavaScript 版（`voyagelight/assets/js/geo.js`）。佈景主題與外掛都以 `tests/geo-cases.json` 中的同一組案例測試這兩份程式：

```sh
php wordpress/tests/geo-test.php
node wordpress/tests/geo-test.js
php wordpress/tests/plugin-guard-test.php
```

最後一個測試以 WordPress 的替身載入外掛，確認 VoyageLight 為佈景主題時它不會加入任何東西、在任何其他佈景主題下會加入它的勾點，以及它的 PHP 名稱都不會與佈景主題的名稱衝突。

## 安裝

在 `wordpress/` 選擇 ZIP 時就選好語言：`-en` 是英文，`-zh-TW` 是繁體中文。佈景主題的文字、自訂器設定與外掛就會是那個語言，不受 **設定 > 一般 > 網站語言** 影響；請把網站語言設成相同語言，WordPress 本身的文字（例如日期與留言表單）才會一致。日後要換語言，上傳另一個 ZIP 即可，設定、選單與小工具都會保留。佈景主題的版本是 `voyagelight/style.css` 裡的版本，外掛的版本是 `picasso-places/picasso-places.php` 裡的版本。兩者都需要 PHP 7.4 或更新版本；佈景主題需要 WordPress 6.2 或更新版本，外掛需要 WordPress 6.4 或更新版本，並已在 WordPress 7.1 搭配 PHP 8.5 上試過。安裝佈景主題，或把外掛與任何其他佈景主題一起安裝。外掛可以與佈景主題同時保持啟用：VoyageLight 或其子佈景主題為使用中的佈景主題時，外掛不會做任何事。

### 佈景主題

1. 在 WordPress 前往 **外觀 > 佈景主題 > 新增 > 上傳佈景主題**，選擇 `voyagelight-<version>-en.zip` 或 `voyagelight-<version>-zh-TW.zip` 並啟用。
2. 到 **設定 > 永久連結** 儲存一次，讓 `/trips/` 與目的地的網址生效。
3. 在 **文章 > 分類**、**文章 > 目的地** 與 **文章 > 景點類型** 建立分類，之後隨時可以改名、移到其他項目之下或刪除。側欄、頁首選單、頁尾與首頁的方塊都會跟著更新。
4. 在 **外觀 > 自訂 > VoyageLight 首頁** 設定首頁的主標題與副標題，並從媒體庫選擇封面照片。留空時，主標題與副標題使用佈景主題自帶的文字（依所選版本的語言），照片也使用佈景主題自帶的照片。同一個區段可以關閉照片下方的相機資訊。同一區段的 **精選專題照片** 設定「Every place has a story」旁的照片；**網站識別** 設定標誌、網站名稱與標語。
5. 在 **外觀 > 自訂 > VoyageLight 文字** 修改佈景主題的任何文字，不需外掛也不需修改檔案：每行一項，寫上網站顯示的文字、`=`，再接新的文字，例如 `最新旅行故事 = 最新文章`。含有 `%s` 或 `%d` 的文字要保留它們；少了或多了的修改會被略過。修改存在網站設定中，更新佈景主題也不會消失。等號後面不寫任何字就會隱藏那段文字，例如 `Powered by WordPress =`。
6. 在 **外觀 > 自訂 > VoyageLight 首頁** 開關首頁的各個區段，在 **區塊順序** 用上移、下移調整它們的順序（大張主圖固定在最上方），並設定顯示的遊記與旅程數量。在 **外觀 > 自訂 > 色彩** 選擇主色：佈景主題會據此產生連結、按鈕、表格與深色模式的色調，並確保清楚可讀。同一處的 **淺色或深色** 設定訪客一開始看到的顏色：依訪客裝置的設定、淺色或深色；**讓訪客切換淺色與深色** 決定是否保留頁首的切換按鈕，訪客的選擇會保存在他們的瀏覽器。
7. 在 **外觀 > 小工具** 把 WordPress 內建的小工具（關於、最新文章、搜尋…）加到 **側欄** 與 **頁尾**；**VoyageLight 面板** 小工具可放回佈景主題自帶的任一面板。區域裡沒有小工具時，會顯示佈景主題原本的內容。WordPress 安裝時自動放進第一個區域的小工具（搜尋、近期文章、近期留言、彙整、分類），會在啟用佈景主題時移到 **未使用的小工具**；若區域裡還有其他小工具則保持不動。要使用它們，拖回去即可。**首頁側欄** 放的是首頁最新遊記旁的內容。
8. 在 **設定 > 永久連結** 的「選填」區，修改旅程（`trips`）、目的地（`destination`）與景點類型（`place-type`）的網址基底，例如 `journeys`。在 **外觀 > 自訂 > VoyageLight 文章頁** 把文章列表與首頁最新遊記旁的側欄放在右側、左側或不顯示；開關標題上方的導覽路徑，以及遊記結尾的相關遊記，並設定相關遊記篇數、頁尾旅程數與卡片摘要長度（以字數計；中文與日文由 WordPress 以字元計算）。

每個頁面也帶有社群分享預覽（Open Graph：標題、摘要與照片）與給搜尋引擎的結構化資料（JSON-LD：遊記與其地點、旅程與其行程，以及導覽路徑）。受密碼保護的文章只分享標題。啟用 SEO 外掛時，佈景主題會把這兩項交給外掛。

### 外掛

1. 在 **外掛 > 安裝外掛 > 上傳外掛** 選擇 `picasso-places-<version>-en.zip` 或 `picasso-places-<version>-zh-TW.zip` 並啟用。
2. 若找不到 `/trips/`，到 **設定 > 永久連結** 儲存一次。
3. 文章內容下方會有 **地理標記與旅遊地圖**，側邊欄會有 **所屬旅程**。旅程有自己的選單。

## 語言

佈景主題與外掛各有英文版與繁體中文版，是分開的 ZIP（見「安裝」），網站擇一使用。文章依撰寫的原文顯示。首頁的目的地磚不論網站語言為何，都會以兩種語言寫出每個地方的名稱。

經營網站不需要下面的步驟，只要選對語言的 ZIP：佈景主題顯示的任何文字，都可以在 **外觀 > 自訂 > VoyageLight 文字** 修改或隱藏（見上方第 5 步）；分類、選單、小工具與文章也都在 WordPress 裡建立。

只有修改佈景主題或外掛的 PHP 時才需要。在 PHP 中新增或修改一段文字後，執行：

```sh
make translations
```

`wordpress/voyagelight` 與 `wordpress/picasso-places` 裡的原始碼沒有 `inc/language.php`，因此會依網站語言顯示，方便試用翻譯。`make translations` 會依 PHP 重新產生 `languages/voyagelight.pot` 與 `languages/picasso-places.pot`，包含每段文字的複數形式與 `translators:` 註解，並同步每個 `.po`：新的文字加上空白的翻譯，PHP 已不再使用的文字則刪除。外掛新增的文字若佈景主題也有，會先填入佈景主題的翻譯，因此兩者用同樣的詞：Journey 作旅程、story 作遊記、stop 作停留點、location 作位置。它會列出尚待翻譯的文字；在 `.po` 填好後執行 `make theme`。只要還有文字沒翻譯，或 `.pot`、`.po` 與 PHP 不一致，建置就會失敗。不需要 gettext 工具或 WP-CLI。

## 顯示的內容

- **首頁：** 一張大照片、各區塊或網站主要分類的照片方塊、最新遊記、旅程、目的地，以及由有位置的文章組成的旅遊地圖。
- **文章：** 位置與停留點清單在文章編輯器中輸入；佈景主題不會從相片讀取這些資料。位置可以是座標、度與分，或 Google 地圖、Apple 地圖、OpenStreetMap、geo: 連結；maps.app.goo.gl 之類的短網址必須先開啟。佈景主題無法讀取的行會保留，並列在編輯器中。**在地圖上選取** 會載入 Leaflet 與 OpenStreetMap 圖磚，**搜尋** 會向 OpenStreetMap Nominatim 查詢，而且只在按下時才進行。支援寬幅與全寬圖片、圖庫與多欄，區塊編輯器會以網站的字型與欄寬顯示。標題下方那一行是文章摘要，只有寫了摘要才會顯示。
- **Picasso 的文章：** 以 **保留相機資訊** 送出的相片，會在下方顯示相機、焦距、光圈、快門速度與 ISO，由 WordPress 從上傳的檔案讀取；圖庫中的相片不會。Picasso 的致謝段落（`picasso-credits`）以較小的字分開顯示。
- **旅程：** 獨立的文章類型，含逐日停留點與所屬遊記，在 WordPress 中以列，或以 `Day | Place | location | note | post ID` 這樣的行輸入。**從所屬遊記加入停留點** 會依連結到這趟旅程的遊記填入各列。Picasso 不會代為填入。
- **REST API：** 文章上的 `_vl_lat`、`_vl_lon`、`_vl_stops` 與 `_vl_trip_id`，以及旅程上的 `_vl_trip_days`，屬於已發佈的文章，能閱讀該文章的人也能讀取這些欄位。寫入需要能編輯該文章的使用者。缺少的座標讀出來是 `0`。送回 `0, 0` 且沒有停留點行，表示沒有位置。帶有這些座標的停留點行，是位於 0, 0 的一個地點。
- **地圖：** 讀者按下 **載入地圖** 之前，不會向其他地方取得任何資料；按下之後，圖磚來自 OpenStreetMap。字型內建於佈景主題。

旅程、目的地、景點類型與地圖短代碼（`[voyagelight_map]` 與 `[voyagelight_atlas]`）屬於佈景主題，因此使用其他佈景主題時會隱藏，換回時會再出現。外掛會在任何佈景主題下恢復旅程與文章欄位。目的地、景點類型與短代碼仍只屬於佈景主題。

## 外掛與佈景主題

Picasso Places 註冊與 VoyageLight 相同的文章欄位、相同的旅程文章類型（`vl_trip`，網址為 `/trips/`）與相同的編輯器方塊，儲存時的檢查也相同，因此文章與旅程可以在兩者之間原樣搬移。使用其他佈景主題時，它會加在單篇文章或旅程頁面的正文之後：

- 有位置或停留點的文章：**這篇遊記的地點** 地圖與停留點清單；
- 連結到已發佈旅程的文章：**所屬旅程：…**；
- 旅程頁面：**旅程地圖**、逐日的 **行程**，以及 **這趟旅程的旅行故事**；
- Picasso 以 **保留相機資訊** 送出的相片下方：相機、焦距、光圈、快門速度與 ISO，精選圖片也包含在內；圖庫中的相片沒有這一行。

它不更動 Feed、摘要、彙整與 REST API，樣式的顏色與字型取自佈景主題。

Picasso 發佈前會先讀取網站的文章接受哪些欄位。有佈景主題或外掛時，它會把故事的地點以這些欄位送出，也可以建立旅程。兩者都沒有時，地點會放在文章結尾，成為一張標有編號圖釘的地圖圖片，以及連到 OpenStreetMap 的清單，文章儲存時也不會建立旅程。

VoyageLight 為使用中的佈景主題時，外掛完全不做任何事，因此兩者可以同時啟用，切換佈景主題也會保留所有內容。外掛比佈景主題先載入，因此不能定義佈景主題已經使用的 PHP 名稱：它的 PHP 位於 `PicassoPlaces` 命名空間，指令碼 handle、表單欄位與 CSS 類別以 `picasso-places` 或 `picasso_places` 開頭。共用指令碼所尋找的 id、類別，以及 `vlGeo`、`vlGeoAdmin` 與 `vlText` 物件，仍沿用 `vl` 名稱。網站沒有位置欄位時，Picasso 自己的文章用 `picasso-places` 與 `picasso-places-map` 標示它所繪的靜態地圖；佈景主題或外掛啟用時，Picasso 改送這些欄位，不再繪製靜態地圖。

位置讀取程式與編輯器、地圖指令碼是同一份共用程式，因此兩者對同一個位置的讀取結果相同。要修改的是佈景主題的檔案；外掛保留副本，因此外掛資料夾可以獨立運作：

| 佈景主題檔案 | 外掛副本 |
| --- | --- |
| `voyagelight/inc/geo.php` | `picasso-places/inc/geo.php`，並加上 `namespace PicassoPlaces;` |
| `voyagelight/assets/js/geo.js` | `picasso-places/assets/js/geo.js` |
| `voyagelight/assets/js/geo-admin.js` | `picasso-places/assets/js/geo-admin.js` |
| `voyagelight/assets/js/map.js` | `picasso-places/assets/js/map.js` |
| `voyagelight/assets/css/geo-admin.css` | `picasso-places/assets/css/geo-admin.css` |
| `voyagelight/assets/vendor/leaflet/` | `picasso-places/assets/vendor/leaflet/` |

改過佈景主題的其中一個檔案後，執行 `python3 scripts/build-theme.py --sync` 更新副本。副本有差異時，`--check`、`make theme` 與 CI 都會失敗。

## 授權

佈景主題為 GPLv2 或更新版本，插圖與示範照片也是；字型採用 SIL Open Font License。`voyagelight/readme.txt` 逐一列出。外掛為 GPLv2 或更新版本；它載入的地圖程式庫 Leaflet 為 BSD-2-Clause。
