//! Interface language. Text is written in English in the code and looked up
//! in `TABLE` for the other languages; `{}` marks a value filled in by `trf`.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    English,
    TraditionalChinese,
    SimplifiedChinese,
    Japanese,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::English, Lang::TraditionalChinese, Lang::SimplifiedChinese, Lang::Japanese];

    /// Name of the language written in that language, for the picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::TraditionalChinese => "繁體中文",
            Lang::SimplifiedChinese => "简体中文",
            Lang::Japanese => "日本語",
        }
    }

    /// Value stored in the `language` setting.
    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::TraditionalChinese => "zh-Hant",
            Lang::SimplifiedChinese => "zh-Hans",
            Lang::Japanese => "ja",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|lang| lang.code() == code)
    }

    /// Reads a system locale such as `zh-TW`, `zh-Hant-HK`, `zh_CN.UTF-8`,
    /// or `ja-JP`. Chinese without a script or region reads as Simplified.
    pub fn from_locale(locale: &str) -> Option<Lang> {
        let tag = locale.split(['.', '@']).next()?.replace('_', "-").to_ascii_lowercase();
        let mut parts = tag.split('-');
        match parts.next()? {
            "en" => Some(Lang::English),
            "ja" => Some(Lang::Japanese),
            "zh" => {
                let traditional = parts.any(|part| matches!(part, "hant" | "tw" | "hk" | "mo"));
                Some(if traditional { Lang::TraditionalChinese } else { Lang::SimplifiedChinese })
            }
            _ => None,
        }
    }

    fn column(self) -> Option<usize> {
        match self {
            Lang::English => None,
            Lang::TraditionalChinese => Some(0),
            Lang::SimplifiedChinese => Some(1),
            Lang::Japanese => Some(2),
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    Lang::ALL[CURRENT.load(Ordering::Relaxed) as usize % Lang::ALL.len()]
}

pub fn set(lang: Lang) {
    let index = Lang::ALL.iter().position(|item| *item == lang).unwrap_or(0);
    CURRENT.store(index as u8, Ordering::Relaxed);
}

/// The first preferred system language Picasso has text for, else English.
pub fn system() -> Lang {
    sys_locale::get_locales().find_map(|locale| Lang::from_locale(&locale)).unwrap_or(Lang::English)
}

/// `text` in the current language. Text without a translation stays English.
pub fn tr(text: &'static str) -> &'static str {
    let Some(column) = current().column() else {
        return text;
    };
    table().get(text).map_or(text, |row| row[column])
}

/// `tr(text)` with each `{}` replaced by the next value in `args`.
pub fn trf(text: &'static str, args: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut parts = tr(text).split("{}").peekable();
    while let Some(part) = parts.next() {
        out.push_str(part);
        if parts.peek().is_some() {
            if let Some(arg) = args.next() {
                out.push_str(&arg.to_string());
            }
        }
    }
    out
}

fn table() -> &'static HashMap<&'static str, [&'static str; 3]> {
    static MAP: OnceLock<HashMap<&'static str, [&'static str; 3]>> = OnceLock::new();
    MAP.get_or_init(|| TABLE.iter().map(|(en, zh_hant, zh_hans, ja)| (*en, [*zh_hant, *zh_hans, *ja])).collect())
}

/// English, Traditional Chinese, Simplified Chinese, Japanese.
const TABLE: &[(&str, &str, &str, &str)] = &[
    // Header and photo tray
    ("Search photos", "搜尋相片", "搜索照片", "写真を検索"),
    ("Slideshow", "投影片播放", "幻灯片放映", "スライドショー"),
    ("Play Fullscreen Slideshow", "全螢幕播放投影片", "全屏播放幻灯片", "全画面でスライドショーを再生"),
    ("Refresh", "重新整理", "刷新", "更新"),
    ("Refresh Thumbnails", "重新整理縮圖", "刷新缩略图", "サムネイルを更新"),
    ("+  Import", "+  匯入", "+  导入", "+  インポート"),
    ("Add Folder to Picasso…", "將資料夾加入 Picasso…", "将文件夹添加到 Picasso…", "Picasso にフォルダを追加…"),
    ("Language", "語言", "语言", "言語"),
    ("System language ({})", "系統語言（{}）", "系统语言（{}）", "システムの言語（{}）"),
    ("Color theme", "色彩主題", "颜色主题", "カラーテーマ"),
    ("System theme", "跟隨系統", "跟随系统", "システムに合わせる"),
    ("Light theme", "淺色主題", "浅色主题", "ライトテーマ"),
    ("Dark theme", "深色主題", "深色主题", "ダークテーマ"),
    ("Switch to the light theme", "切換為淺色主題", "切换为浅色主题", "ライトテーマに切り替え"),
    ("Switch to the dark theme", "切換為深色主題", "切换为深色主题", "ダークテーマに切り替え"),
    ("Photo Tray", "相片匣", "照片托盘", "フォトトレイ"),
    ("Clear", "清除", "清除", "クリア"),
    ("Clear items from the selection", "清除選取的項目", "清除所选项目", "選択を解除"),
    ("Delete", "刪除", "删除", "削除"),
    ("{} photo", "{} 張相片", "{} 张照片", "{} 枚の写真"),
    ("{} photos", "{} 張相片", "{} 张照片", "{} 枚の写真"),
    ("Show or hide the map of the selected photo", "顯示或隱藏所選相片的地圖", "显示或隐藏所选照片的地图", "選択した写真の地図を表示/非表示"),
    (
        "Show or hide photo properties and EXIF (I)",
        "顯示或隱藏相片內容與 EXIF (I)",
        "显示或隐藏照片属性和 EXIF (I)",
        "写真のプロパティと EXIF を表示/非表示 (I)",
    ),
    ("Scanning {}", "正在掃描 {}", "正在扫描 {}", "{} をスキャン中"),
    (
        "Scanning {} · {} checked · {} updated",
        "正在掃描 {} · 已檢查 {} · 已更新 {}",
        "正在扫描 {} · 已检查 {} · 已更新 {}",
        "{} をスキャン中 · {} 件確認 · {} 件更新",
    ),
    ("{} files could not be read", "有 {} 個檔案無法讀取", "有 {} 个文件无法读取", "{} 個のファイルを読み込めませんでした"),
    ("Can't watch {}: {}", "無法監看 {}：{}", "无法监视 {}：{}", "{} を監視できません: {}"),
    // Sidebar
    ("Library", "圖庫", "图库", "ライブラリ"),
    ("Starred Photos", "已加星號的相片", "已加星标的照片", "スター付きの写真"),
    ("Places", "地點", "地点", "撮影地"),
    ("Albums", "相簿", "相册", "アルバム"),
    ("Delete album", "刪除相簿", "删除相册", "アルバムを削除"),
    ("+  New album", "+  新增相簿", "+  新建相册", "+  新しいアルバム"),
    ("Folders", "資料夾", "文件夹", "フォルダ"),
    ("Show in Explorer", "在檔案總管中顯示", "在资源管理器中显示", "エクスプローラーで表示"),
    ("Show in Finder", "在 Finder 中顯示", "在 Finder 中显示", "Finder で表示"),
    ("Remove from Picasso…", "從 Picasso 移除…", "从 Picasso 移除…", "Picasso から削除…"),
    // Help
    (
        "A local photo library. Folders are scanned on this computer.",
        "本機相片圖庫。資料夾只會在這台電腦上掃描。",
        "本地照片图库。文件夹只在这台电脑上扫描。",
        "ローカルの写真ライブラリです。フォルダはこのコンピューター上でスキャンされます。",
    ),
    ("Library: {}", "圖庫：{}", "图库：{}", "ライブラリ: {}"),
    ("Enter — open    Esc — back", "Enter — 開啟    Esc — 返回", "Enter — 打开    Esc — 返回", "Enter — 開く    Esc — 戻る"),
    ("Arrows — move or previous / next", "方向鍵 — 移動，或上一張／下一張", "方向键 — 移动，或上一张／下一张", "矢印キー — 移動、前へ／次へ"),
    (
        "Space — slideshow or video     M — mute     S — star",
        "Space — 投影片或影片     M — 靜音     S — 星號",
        "Space — 幻灯片或视频     M — 静音     S — 星标",
        "Space — スライドショー／動画     M — ミュート     S — スター",
    ),
    (
        "Places — geotag on the OpenStreetMap",
        "地點 — 在 OpenStreetMap 上標記地點",
        "地点 — 在 OpenStreetMap 上添加地理标记",
        "撮影地 — OpenStreetMap 上でジオタグを付ける",
    ),
    (
        "HEIC, camera RAW (including NEF and 3FR), and video",
        "支援 HEIC、相機 RAW（包括 NEF 和 3FR）與影片",
        "支持 HEIC、相机 RAW（包括 NEF 和 3FR）和视频",
        "HEIC、カメラ RAW（NEF、3FR を含む）、動画に対応",
    ),
    (
        "Ctrl+] or R — rotate clockwise     Ctrl+[ or Shift+R — counterclockwise",
        "Ctrl+] 或 R — 順時針旋轉     Ctrl+[ 或 Shift+R — 逆時針旋轉",
        "Ctrl+] 或 R — 顺时针旋转     Ctrl+[ 或 Shift+R — 逆时针旋转",
        "Ctrl+] または R — 右に回転     Ctrl+[ または Shift+R — 左に回転",
    ),
    (
        "Rotation is saved in the library; photo files are never changed",
        "旋轉只儲存在圖庫中，不會修改相片檔案",
        "旋转只保存在图库中，不会修改照片文件",
        "回転はライブラリに保存され、写真ファイルは変更されません",
    ),
    ("I — details, histogram, and EXIF", "I — 詳細資料、色階分佈圖與 EXIF", "I — 详细信息、直方图和 EXIF", "I — 詳細、ヒストグラム、EXIF"),
    ("+ / − zoom     0 fit     1 actual size", "+ / − 縮放     0 符合視窗     1 實際大小", "+ / − 缩放     0 适合窗口     1 实际大小", "+ / − ズーム     0 全体表示     1 実寸"),
    (
        "Delete — trash     F5 — rescan     F11 — fullscreen",
        "Delete — 移至垃圾桶     F5 — 重新掃描     F11 — 全螢幕",
        "Delete — 移到回收站     F5 — 重新扫描     F11 — 全屏",
        "Delete — ごみ箱へ     F5 — 再スキャン     F11 — 全画面",
    ),
    // Dialogs
    ("Add folder", "新增資料夾", "添加文件夹", "フォルダを追加"),
    (
        "Picasso watches this folder and adds new photos as they appear.",
        "Picasso 會監看這個資料夾，並自動加入新的相片。",
        "Picasso 会监视这个文件夹，并自动添加新照片。",
        "Picasso はこのフォルダを監視し、新しい写真を自動的に追加します。",
    ),
    ("Browse", "瀏覽", "浏览", "参照"),
    ("Cancel", "取消", "取消", "キャンセル"),
    ("Add a photo folder", "新增相片資料夾", "添加照片文件夹", "写真フォルダを追加"),
    ("New album", "新增相簿", "新建相册", "新しいアルバム"),
    ("Album name", "相簿名稱", "相册名称", "アルバム名"),
    ("Create", "建立", "创建", "作成"),
    ("Move to trash", "移至垃圾桶", "移到回收站", "ごみ箱に移動"),
    ("Move 1 photo to the system trash?", "要將 1 張相片移至垃圾桶嗎？", "要将 1 张照片移到回收站吗？", "1 枚の写真をごみ箱に移動しますか？"),
    ("Move {} photos to the system trash?", "要將 {} 張相片移至垃圾桶嗎？", "要将 {} 张照片移到回收站吗？", "{} 枚の写真をごみ箱に移動しますか？"),
    ("Search “{}”", "搜尋「{}」", "搜索“{}”", "「{}」の検索結果"),
    ("Folder", "資料夾", "文件夹", "フォルダ"),
    ("Album", "相簿", "相册", "アルバム"),
    ("Rotated {} photo clockwise", "已順時針旋轉 {} 張相片", "已顺时针旋转 {} 张照片", "{} 枚の写真を右に回転しました"),
    ("Rotated {} photos clockwise", "已順時針旋轉 {} 張相片", "已顺时针旋转 {} 张照片", "{} 枚の写真を右に回転しました"),
    ("Rotated {} photo counterclockwise", "已逆時針旋轉 {} 張相片", "已逆时针旋转 {} 张照片", "{} 枚の写真を左に回転しました"),
    ("Rotated {} photos counterclockwise", "已逆時針旋轉 {} 張相片", "已逆时针旋转 {} 张照片", "{} 枚の写真を左に回転しました"),
    ("Couldn't trash {}: {}", "無法將 {} 移至垃圾桶：{}", "无法将 {} 移到回收站：{}", "{} をごみ箱に移動できませんでした: {}"),
    ("Moved {} to trash", "已將 {} 個項目移至垃圾桶", "已将 {} 个项目移到回收站", "{} 件をごみ箱に移動しました"),
    // Library grid and toolbar
    ("Sort by Date", "依日期排序", "按日期排序", "日付順"),
    ("Sort by Date, oldest", "依日期排序（最舊優先）", "按日期排序（最早优先）", "日付順（古い順）"),
    ("Sort by Name", "依名稱排序", "按名称排序", "名前順"),
    ("Select starred photos", "選取已加星號的相片", "选择已加星标的照片", "スター付きの写真を選択"),
    ("Unstar", "移除星號", "取消星标", "スターを外す"),
    ("Star", "加上星號", "加星标", "スターを付ける"),
    ("Rotate Right", "向右旋轉", "向右旋转", "右に回転"),
    ("Rotate clockwise (Ctrl+])", "順時針旋轉 (Ctrl+])", "顺时针旋转 (Ctrl+])", "右に回転 (Ctrl+])"),
    ("Rotate Left", "向左旋轉", "向左旋转", "左に回転"),
    ("Rotate counterclockwise (Ctrl+[)", "逆時針旋轉 (Ctrl+[)", "逆时针旋转 (Ctrl+[)", "左に回転 (Ctrl+[)"),
    ("Remove from Album", "從相簿中移除", "从相册中移除", "アルバムから削除"),
    ("Add to Album", "加入相簿", "添加到相册", "アルバムに追加"),
    ("New album…", "新增相簿…", "新建相册…", "新しいアルバム…"),
    ("Added {} to album", "已將 {} 張加入相簿", "已将 {} 张添加到相册", "{} 枚をアルバムに追加しました"),
    (
        "Geotagged in Picasso — show on map",
        "已在 Picasso 中標記地點 — 在地圖上顯示",
        "已在 Picasso 中添加地理标记 — 在地图上显示",
        "Picasso でジオタグ済み — 地図に表示",
    ),
    ("Has GPS location — show on map", "含有 GPS 位置 — 在地圖上顯示", "包含 GPS 位置 — 在地图上显示", "GPS 位置あり — 地図に表示"),
    ("Open", "開啟", "打开", "開く"),
    ("Rotate Clockwise", "順時針旋轉", "顺时针旋转", "右に回転"),
    ("Rotate Counterclockwise", "逆時針旋轉", "逆时针旋转", "左に回転"),
    ("Show in folder", "在資料夾中顯示", "在文件夹中显示", "フォルダで表示"),
    ("Show on map", "在地圖上顯示", "在地图上显示", "地図で表示"),
    ("No photos found", "找不到相片", "未找到照片", "写真が見つかりません"),
    ("No results found", "找不到符合的結果", "未找到结果", "一致する結果はありません"),
    (
        "Import a folder and Picasso scans it for pictures.",
        "匯入資料夾，Picasso 就會掃描其中的相片。",
        "导入文件夹，Picasso 会扫描其中的照片。",
        "フォルダをインポートすると、Picasso が写真をスキャンします。",
    ),
    ("+  Import folder", "+  匯入資料夾", "+  导入文件夹", "+  フォルダをインポート"),
    // Info and Map panel
    ("Info", "資訊", "信息", "情報"),
    ("Map", "地圖", "地图", "地図"),
    ("Close panel", "關閉面板", "关闭面板", "パネルを閉じる"),
    ("Select a photo to see its details.", "選取相片以查看詳細資料。", "选择照片以查看详细信息。", "写真を選択すると詳細が表示されます。"),
    ("Properties", "內容", "属性", "プロパティ"),
    ("Taken", "拍攝時間", "拍摄时间", "撮影日時"),
    ("Dimensions", "尺寸", "尺寸", "サイズ"),
    ("File size", "檔案大小", "文件大小", "ファイルサイズ"),
    ("Modified", "修改時間", "修改时间", "更新日時"),
    ("Duration", "長度", "时长", "再生時間"),
    ("Camera", "相機", "相机", "カメラ"),
    ("Rotated", "旋轉", "旋转", "回転"),
    ("{}° clockwise", "順時針 {}°", "顺时针 {}°", "右に {}°"),
    ("Library folder", "圖庫資料夾", "图库文件夹", "ライブラリのフォルダ"),
    ("Location", "位置", "位置", "位置"),
    ("pinned", "手動標記", "手动标记", "手動"),
    (
        "{} photos selected · showing one",
        "已選取 {} 張相片 · 顯示其中一張",
        "已选择 {} 张照片 · 显示其中一张",
        "{} 枚選択中 · 1 枚を表示",
    ),
    ("Video", "影片", "视频", "動画"),
    ("Camera EXIF", "相機 EXIF", "相机 EXIF", "カメラ EXIF"),
    ("Reading EXIF…", "正在讀取 EXIF…", "正在读取 EXIF…", "EXIF を読み込み中…"),
    ("No EXIF data in this file", "此檔案沒有 EXIF 資料", "此文件没有 EXIF 数据", "このファイルには EXIF データがありません"),
    ("Show map", "顯示地圖", "显示地图", "地図を表示"),
    ("No GPS location", "沒有 GPS 位置", "没有 GPS 位置", "GPS 位置情報なし"),
    (
        "This photo's EXIF has no GPS coordinates.",
        "這張相片的 EXIF 沒有 GPS 座標。",
        "这张照片的 EXIF 中没有 GPS 坐标。",
        "この写真の EXIF には GPS 座標がありません。",
    ),
    ("Geotag it in Places", "在「地點」中標記", "在“地点”中添加地理标记", "撮影地でジオタグを付ける"),
    ("Pinned in Picasso", "在 Picasso 中標記", "在 Picasso 中标记", "Picasso で設定"),
    ("From camera GPS", "來自相機 GPS", "来自相机 GPS", "カメラの GPS"),
    ("Open in OpenStreetMap", "在 OpenStreetMap 中開啟", "在 OpenStreetMap 中打开", "OpenStreetMap で開く"),
    ("Show in Places", "在「地點」中顯示", "在“地点”中显示", "撮影地で表示"),
    // Viewer
    ("{} of {}", "第 {} 張，共 {} 張", "第 {} 张，共 {} 张", "{} / {}"),
    ("Play / Pause (Space)", "播放／暫停 (Space)", "播放／暂停 (Space)", "再生／一時停止 (Space)"),
    ("Mute (M)", "靜音 (M)", "静音 (M)", "ミュート (M)"),
    ("‹  Library", "‹  圖庫", "‹  图库", "‹  ライブラリ"),
    ("Return to organized thumbnails", "返回縮圖檢視", "返回缩略图视图", "サムネイル一覧に戻る"),
    ("Exit Slideshow", "結束投影片播放", "退出幻灯片放映", "スライドショーを終了"),
    ("Details, histogram, and EXIF (I)", "詳細資料、色階分佈圖與 EXIF (I)", "详细信息、直方图和 EXIF (I)", "詳細、ヒストグラム、EXIF (I)"),
    ("Open map", "開啟地圖", "打开地图", "地図を開く"),
    (
        "Open this photo in OpenStreetMap",
        "在 OpenStreetMap 中開啟這張相片的位置",
        "在 OpenStreetMap 中打开这张照片的位置",
        "この写真の位置を OpenStreetMap で開く",
    ),
    ("Fit", "符合視窗", "适合窗口", "全体表示"),
    ("Add/Remove Star", "加上／移除星號", "添加／取消星标", "スターを付ける／外す"),
    ("Pause", "暫停", "暂停", "一時停止"),
    ("Play", "播放", "播放", "再生"),
    ("Loading…", "載入中…", "加载中…", "読み込み中…"),
    ("Histogram", "色階分佈圖", "直方图", "ヒストグラム"),
    ("Details", "詳細資料", "详细信息", "詳細"),
    ("File", "檔案", "文件", "ファイル"),
    ("Size", "大小", "大小", "サイズ"),
    ("{}° clockwise in Picasso", "在 Picasso 中順時針旋轉 {}°", "在 Picasso 中顺时针旋转 {}°", "Picasso で右に {}°"),
    ("Place", "地點", "地点", "場所"),
    ("Caption", "說明", "说明", "キャプション"),
    (
        "Shadows clipped {}%   Highlights clipped {}%",
        "暗部裁切 {}%   亮部裁切 {}%",
        "暗部溢出 {}%   高光溢出 {}%",
        "シャドウのクリップ {}%   ハイライトのクリップ {}%",
    ),
    (
        "Level {}\nLuminance {}%\nRed {}%  Green {}%  Blue {}%",
        "色階 {}\n亮度 {}%\n紅 {}%  綠 {}%  藍 {}%",
        "色阶 {}\n亮度 {}%\n红 {}%  绿 {}%  蓝 {}%",
        "レベル {}\n輝度 {}%\n赤 {}%  緑 {}%  青 {}%",
    ),
    // Places
    ("Click the map…", "點選地圖…", "点击地图…", "地図をクリック…"),
    ("Geotag", "標記地點", "地理标记", "ジオタグ"),
    ("Place the selected photos on the map", "將所選相片標記在地圖上", "将所选照片放到地图上", "選択した写真を地図に配置"),
    ("Select photos, then Geotag", "請先選取相片，再按「標記地點」", "请先选择照片，再点“地理标记”", "写真を選択してから「ジオタグ」を押してください"),
    ("Clear geotag", "清除地點標記", "清除地理标记", "ジオタグを解除"),
    (
        "Remove the pin you placed. Camera GPS stays.",
        "移除你放置的圖釘，相機 GPS 會保留。",
        "移除你放置的图钉，相机 GPS 会保留。",
        "配置したピンを削除します。カメラの GPS は残ります。",
    ),
    ("Cleared geotag", "已清除地點標記", "已清除地理标记", "ジオタグを解除しました"),
    ("Open this view in OpenStreetMap", "在 OpenStreetMap 中開啟此檢視", "在 OpenStreetMap 中打开此视图", "この表示を OpenStreetMap で開く"),
    ("Geotagged {} photo", "已標記 {} 張相片的地點", "已为 {} 张照片添加地理标记", "{} 枚の写真にジオタグを付けました"),
    ("Geotagged {} photos", "已標記 {} 張相片的地點", "已为 {} 张照片添加地理标记", "{} 枚の写真にジオタグを付けました"),
    (
        "No geotagged photos yet. Select photos, press Geotag, then click the map.",
        "還沒有標記地點的相片。選取相片，按「標記地點」，再點選地圖。",
        "还没有带地理标记的照片。选择照片，点“地理标记”，然后点击地图。",
        "ジオタグ付きの写真はまだありません。写真を選択し、「ジオタグ」を押してから地図をクリックしてください。",
    ),
    // About
    ("About Picasso", "關於 Picasso", "关于 Picasso", "Picasso について"),
    ("Close", "關閉", "关闭", "閉じる"),
    ("(Build {})", "（建置 {}）", "（构建 {}）", "（ビルド {}）"),
    ("Designed for speed and clarity", "為速度與清晰而設計", "为速度与清晰而设计", "速さと見やすさのために"),
    (
        "A fast local photo library in the spirit of Picasa",
        "承襲 Picasa 精神的快速本機相片圖庫",
        "延续 Picasa 精神的快速本地照片图库",
        "Picasa の精神を受け継ぐ高速なローカル写真ライブラリ",
    ),
    ("Key features", "主要功能", "主要功能", "主な機能"),
    (
        "Fast thumbnails and background scanning, written in Rust",
        "以 Rust 打造的快速縮圖與背景掃描",
        "基于 Rust 的快速缩略图与后台扫描",
        "Rust による高速なサムネイルとバックグラウンドスキャン",
    ),
    (
        "Timeline, folder tree, albums, and places on a map",
        "時間軸、資料夾樹狀圖、相簿與地圖上的地點",
        "时间线、文件夹树、相册和地图上的地点",
        "タイムライン、フォルダーツリー、アルバム、地図上の撮影地",
    ),
    (
        "Non-destructive editing with undo and redo",
        "可復原與重做的非破壞性編輯",
        "可撤销和重做的非破坏性编辑",
        "元に戻す・やり直しができる非破壊編集",
    ),
    (
        "HEIC, camera RAW, and video playback",
        "支援 HEIC、相機 RAW 與影片播放",
        "支持 HEIC、相机 RAW 和视频播放",
        "HEIC、カメラ RAW、動画再生に対応",
    ),
    ("Credits", "致謝", "致谢", "クレジット"),
    (
        "Built with Rust, egui, image, rawler, heic-rs, rusqlite, rodio, and FFmpeg",
        "使用 Rust、egui、image、rawler、heic-rs、rusqlite、rodio 與 FFmpeg 打造",
        "使用 Rust、egui、image、rawler、heic-rs、rusqlite、rodio 和 FFmpeg 构建",
        "Rust、egui、image、rawler、heic-rs、rusqlite、rodio、FFmpeg で構築",
    ),
    (
        "Map data © OpenStreetMap contributors",
        "地圖資料 © OpenStreetMap 貢獻者",
        "地图数据 © OpenStreetMap 贡献者",
        "地図データ © OpenStreetMap 寄稿者",
    ),
    (
        "And everyone who contributes to these open source projects",
        "以及所有為這些開放原始碼專案貢獻的人",
        "以及所有为这些开源项目做出贡献的人",
        "そして、これらのオープンソースプロジェクトに貢献するすべての方々",
    ),
    ("License", "授權", "许可证", "ライセンス"),
    (
        "© {} Picasso contributors. Licensed under the {} License.",
        "© {} Picasso 貢獻者。依 {} 授權條款發布。",
        "© {} Picasso 贡献者。依据 {} 许可证发布。",
        "© {} Picasso 寄稿者。{} ライセンスで公開。",
    ),
    // Editor
    ("Edit", "編輯", "编辑", "編集"),
    (
        "Adjust light and color without changing the original (E)",
        "調整光線與色彩，不會修改原始檔案 (E)",
        "调整光线和色彩，不会修改原始文件 (E)",
        "元のファイルを変更せずに明るさと色を調整 (E)",
    ),
    ("Done", "完成", "完成", "完了"),
    ("Light", "光線", "光线", "ライト"),
    ("Color", "色彩", "色彩", "カラー"),
    ("Effects", "效果", "效果", "効果"),
    ("Exposure", "曝光度", "曝光度", "露光量"),
    ("Contrast", "對比", "对比度", "コントラスト"),
    ("Highlights", "亮部", "高光", "ハイライト"),
    ("Shadows", "陰影", "阴影", "シャドウ"),
    ("Whites", "白色", "白色", "白レベル"),
    ("Blacks", "黑色", "黑色", "黒レベル"),
    ("Temperature", "色溫", "色温", "色温度"),
    ("Tint", "色調", "色调", "色かぶり補正"),
    ("Vibrance", "細節飽和度", "自然饱和度", "自然な彩度"),
    ("Saturation", "飽和度", "饱和度", "彩度"),
    ("Black & White", "黑白", "黑白", "白黒"),
    ("Vignette", "暈映", "暗角", "周辺光量"),
    ("Grain", "顆粒", "颗粒", "粒子"),
    ("On", "開", "开", "オン"),
    ("Off", "關", "关", "オフ"),
    ("Reset all", "全部重設", "全部重置", "すべてリセット"),
    ("Undo", "復原", "撤销", "元に戻す"),
    ("Redo", "重做", "重做", "やり直す"),
    ("Undo all", "全部復原", "全部撤销", "すべて戻す"),
    ("Redo all", "全部重做", "全部重做", "すべてやり直す"),
    ("Undo (Ctrl+Z)", "復原 (Ctrl+Z)", "撤销 (Ctrl+Z)", "元に戻す (Ctrl+Z)"),
    ("Redo (Ctrl+Y)", "重做 (Ctrl+Y)", "重做 (Ctrl+Y)", "やり直す (Ctrl+Y)"),
    ("Back to the original", "回到原始相片", "回到原始照片", "元の写真に戻す"),
    ("Apply every step again", "重新套用所有步驟", "重新应用所有步骤", "すべての手順をもう一度適用"),
    ("Before", "修改前", "修改前", "補正前"),
    ("Show the original (\\)", "顯示原始相片 (\\)", "显示原始照片 (\\)", "元の写真を表示 (\\)"),
    ("Double-click to reset", "按兩下即可重設", "双击可重置", "ダブルクリックでリセット"),
    ("History", "步驟記錄", "历史记录", "履歴"),
    ("Original", "原始相片", "原始照片", "元の写真"),
    (
        "Edits are saved as steps in the library. The original file is never changed.",
        "編輯會以步驟儲存在圖庫中，原始檔案永遠不會被修改。",
        "编辑以步骤形式保存在图库中，原始文件永远不会被修改。",
        "編集は手順としてライブラリに保存され、元のファイルは変更されません。",
    ),
    ("Export…", "匯出…", "导出…", "書き出し…"),
    ("Save the edited photo as a new file", "將編輯後的相片另存為新檔案", "将编辑后的照片另存为新文件", "編集した写真を新しいファイルとして保存"),
    ("Export edited photo", "匯出編輯後的相片", "导出编辑后的照片", "編集した写真を書き出し"),
    ("Exporting {}…", "正在匯出 {}…", "正在导出 {}…", "{} を書き出し中…"),
    ("Exported {}", "已匯出 {}", "已导出 {}", "{} を書き出しました"),
    ("Couldn't export: {}", "無法匯出：{}", "无法导出：{}", "書き出せませんでした: {}"),
    ("Choose a different name than the original", "請選擇與原始檔案不同的名稱", "请选择与原始文件不同的名称", "元のファイルとは別の名前を選んでください"),
    ("Videos can't be edited", "影片無法編輯", "视频无法编辑", "動画は編集できません"),
    (
        "E — edit     Ctrl+Z / Ctrl+Y — undo / redo     \\ — before",
        "E — 編輯     Ctrl+Z / Ctrl+Y — 復原／重做     \\ — 修改前",
        "E — 编辑     Ctrl+Z / Ctrl+Y — 撤销／重做     \\ — 修改前",
        "E — 編集     Ctrl+Z / Ctrl+Y — 元に戻す／やり直す     \\ — 補正前",
    ),
    (
        "Y — side by side     Shift+Y — split     J — clipping     C — crop",
        "Y — 並排     Shift+Y — 分割     J — 剪裁警告     C — 裁切",
        "Y — 并排     Shift+Y — 分割     J — 溢出警告     C — 裁剪",
        "Y — 左右に並べる     Shift+Y — 分割     J — クリッピング     C — 切り抜き",
    ),
    (
        "Ctrl+Shift+C / Ctrl+Shift+V — copy / paste edit settings",
        "Ctrl+Shift+C / Ctrl+Shift+V — 複製／貼上編輯設定",
        "Ctrl+Shift+C / Ctrl+Shift+V — 复制／粘贴编辑设置",
        "Ctrl+Shift+C / Ctrl+Shift+V — 編集設定のコピー／ペースト",
    ),
    // Develop tools
    ("Adjust", "調整", "调整", "調整"),
    ("Crop", "裁切", "裁剪", "切り抜き"),
    ("Masks", "遮罩", "蒙版", "マスク"),
    ("Light, color, effects, and detail", "光線、色彩、效果與細節", "光线、色彩、效果与细节", "ライト、カラー、効果、ディテール"),
    ("Crop & straighten (C)", "裁切與拉直 (C)", "裁剪与拉直 (C)", "切り抜きと角度補正 (C)"),
    ("Crop & straighten", "裁切與拉直", "裁剪与拉直", "切り抜きと角度補正"),
    ("Linear and radial local adjustments", "線性與放射狀局部調整", "线性与径向局部调整", "線形・円形の部分補正"),
    ("After", "修改後", "修改后", "補正後"),
    ("Side by side", "並排", "并排", "並べて表示"),
    ("Split", "分割", "分割", "分割"),
    ("Clipping", "剪裁警告", "溢出警告", "クリッピング"),
    ("Before and after side by side (Y)", "並排比較修改前後 (Y)", "并排比较修改前后 (Y)", "補正前と補正後を並べて表示 (Y)"),
    ("Before and after split view (Shift+Y)", "分割比較修改前後 (Shift+Y)", "分割比较修改前后 (Shift+Y)", "補正前後を分割表示 (Shift+Y)"),
    ("Show clipped highlights and shadows (J)", "顯示過曝與欠曝區域 (J)", "显示过曝与欠曝区域 (J)", "白飛びと黒つぶれを表示 (J)"),
    ("Auto", "自動", "自动", "自動"),
    ("Auto tone: balance exposure and contrast", "自動色調：平衡曝光度與對比", "自动色调：平衡曝光度与对比度", "自動補正: 露光量とコントラストを整える"),
    ("Back to the untouched photo, as a step", "回到未修改的相片（記錄為一個步驟）", "回到未修改的照片（记录为一个步骤）", "未補正の写真に戻す（ステップとして記録）"),
    ("Copy", "複製", "复制", "コピー"),
    ("Paste", "貼上", "粘贴", "ペースト"),
    ("Copy these settings (Ctrl+Shift+C)", "複製這些設定 (Ctrl+Shift+C)", "复制这些设置 (Ctrl+Shift+C)", "この設定をコピー (Ctrl+Shift+C)"),
    ("Paste copied settings (Ctrl+Shift+V)", "貼上複製的設定 (Ctrl+Shift+V)", "粘贴复制的设置 (Ctrl+Shift+V)", "コピーした設定をペースト (Ctrl+Shift+V)"),
    ("Paste settings", "貼上設定", "粘贴设置", "設定をペースト"),
    ("Copy edit settings", "複製編輯設定", "复制编辑设置", "編集設定をコピー"),
    ("Edited — open in the editor", "已編輯 — 在編輯器中開啟", "已编辑 — 在编辑器中打开", "編集済み — エディターで開く"),
    ("Edit settings copied", "已複製編輯設定", "已复制编辑设置", "編集設定をコピーしました"),
    ("Copy an edit first (Ctrl+Shift+C)", "請先複製編輯設定 (Ctrl+Shift+C)", "请先复制编辑设置 (Ctrl+Shift+C)", "先に編集設定をコピーしてください (Ctrl+Shift+C)"),
    ("Pasted the edit into {} photos", "已將編輯貼到 {} 張相片", "已将编辑粘贴到 {} 张照片", "{} 枚の写真に編集をペーストしました"),
    ("The edit couldn't be rendered", "無法套用此編輯", "无法渲染此编辑", "編集を描画できませんでした"),
    // Develop sections and sliders
    ("Profile", "描述檔", "配置文件", "プロファイル"),
    ("Presets", "預設集", "预设", "プリセット"),
    ("Tone curve", "色調曲線", "色调曲线", "トーンカーブ"),
    ("Presence", "外觀", "外观", "外観"),
    ("Color mixer", "混色器", "混色器", "カラーミキサー"),
    ("Color grading", "色彩分級", "颜色分级", "カラーグレーディング"),
    ("Detail", "細節", "细节", "ディテール"),
    ("Texture", "紋理", "纹理", "テクスチャ"),
    ("Clarity", "清晰度", "清晰度", "明瞭度"),
    ("Dehaze", "去朦朧", "去朦胧", "かすみの除去"),
    ("Sharpening", "銳利化", "锐化", "シャープ"),
    ("Noise reduction", "雜訊減少", "减少杂色", "ノイズ軽減"),
    ("Convert to black & white", "轉換為黑白", "转换为黑白", "白黒に変換"),
    ("Amount", "總量", "数量", "適用量"),
    ("Midpoint", "中點", "中点", "中心点"),
    ("Roundness", "圓度", "圆度", "丸み"),
    ("Feather", "羽化", "羽化", "ぼかし"),
    ("Roughness", "粗糙度", "粗糙度", "粗さ"),
    ("Radius", "半徑", "半径", "半径"),
    ("Masking", "遮色", "蒙版", "マスク"),
    ("Luminance", "明亮度", "明亮度", "輝度"),
    ("Hue", "色相", "色相", "色相"),
    ("Straighten", "拉直", "拉直", "角度補正"),
    ("Flip horizontal", "水平翻轉", "水平翻转", "左右反転"),
    ("Flip vertical", "垂直翻轉", "垂直翻转", "上下反転"),
    ("Crop left", "裁切左側", "裁剪左侧", "切り抜き（左）"),
    ("Crop top", "裁切上方", "裁剪上方", "切り抜き（上）"),
    ("Crop right", "裁切右側", "裁剪右侧", "切り抜き（右）"),
    ("Crop bottom", "裁切下方", "裁剪下方", "切り抜き（下）"),
    ("Lights", "亮色調", "亮调", "ライト"),
    ("Darks", "暗色調", "暗调", "ダーク"),
    ("Shadow split", "陰影分割點", "阴影分割点", "シャドウ境界"),
    ("Midtone split", "中間調分割點", "中间调分割点", "中間調境界"),
    ("Highlight split", "亮部分割點", "高光分割点", "ハイライト境界"),
    ("Splits", "分割點", "分割点", "境界"),
    ("Parametric", "參數", "参数", "パラメトリック"),
    ("Point", "點", "点", "ポイント"),
    ("Click to add a point, drag to move, double-click to remove.", "按一下新增控制點，拖曳移動，按兩下移除。", "单击添加控制点，拖动移动，双击删除。", "クリックで点を追加、ドラッグで移動、ダブルクリックで削除。"),
    ("Reset curve", "重設曲線", "重置曲线", "カーブをリセット"),
    ("Red", "紅色", "红色", "レッド"),
    ("Orange", "橙色", "橙色", "オレンジ"),
    ("Yellow", "黃色", "黄色", "イエロー"),
    ("Green", "綠色", "绿色", "グリーン"),
    ("Aqua", "淺綠色", "浅绿色", "アクア"),
    ("Blue", "藍色", "蓝色", "ブルー"),
    ("Purple", "紫色", "紫色", "パープル"),
    ("Magenta", "洋紅色", "洋红色", "マゼンタ"),
    ("Midtones", "中間調", "中间调", "中間調"),
    ("Global", "全域", "全局", "全体"),
    ("Blending", "混合", "混合", "ブレンド"),
    ("Balance", "平衡", "平衡", "バランス"),
    ("B&W mix", "黑白混合", "黑白混合", "白黒ミックス"),
    ("Hue {}°  Saturation {}\nDouble-click to reset", "色相 {}°  飽和度 {}\n按兩下重設", "色相 {}°  饱和度 {}\n双击重置", "色相 {}°  彩度 {}\nダブルクリックでリセット"),
    ("{} adjustments", "{} 項調整", "{} 项调整", "{} 件の補正"),
    // Profiles, white balance, and presets
    ("Neutral", "中性", "中性", "ニュートラル"),
    ("Vivid", "鮮豔", "鲜艳", "ビビッド"),
    ("Landscape", "風景", "风景", "風景"),
    ("Portrait", "人像", "人像", "ポートレート"),
    ("Monochrome", "單色", "单色", "モノクロ"),
    ("Profile: {}", "描述檔：{}", "配置文件：{}", "プロファイル: {}"),
    ("Choose…", "選擇…", "选择…", "選択…"),
    ("As shot", "原照設定", "原照设置", "撮影時の設定"),
    ("Daylight", "日光", "日光", "太陽光"),
    ("Cloudy", "陰天", "阴天", "曇天"),
    ("Shade", "陰影", "阴影", "日陰"),
    ("Tungsten", "鎢絲燈", "钨丝灯", "白熱電球"),
    ("Fluorescent", "螢光燈", "荧光灯", "蛍光灯"),
    ("White balance: {}", "白平衡：{}", "白平衡：{}", "ホワイトバランス: {}"),
    ("Auto tone", "自動色調", "自动色调", "自動補正"),
    ("Auto white balance", "自動白平衡", "自动白平衡", "自動ホワイトバランス"),
    ("White balance picker", "白平衡滴管", "白平衡吸管", "ホワイトバランス スポイト"),
    ("White balance picker: click something that should be neutral gray", "白平衡滴管：點選應為中性灰的位置", "白平衡吸管：点击应为中性灰的位置", "ホワイトバランス スポイト: 本来グレーであるべき部分をクリック"),
    ("Click something that should be neutral gray", "點選應為中性灰的位置", "点击应为中性灰的位置", "本来グレーであるべき部分をクリック"),
    ("B&W", "黑白", "黑白", "白黒"),
    ("Film", "底片", "胶片", "フィルム"),
    ("Style", "風格", "风格", "スタイル"),
    ("Preset: {}", "預設集：{}", "预设：{}", "プリセット: {}"),
    ("Vivid Pop", "鮮明亮麗", "鲜明亮丽", "ビビッドポップ"),
    ("Golden Hour", "黃金時刻", "黄金时刻", "ゴールデンアワー"),
    ("Cool Breeze", "清涼微風", "清凉微风", "クールブリーズ"),
    ("Teal & Orange", "青橙色調", "青橙色调", "ティール＆オレンジ"),
    ("Soft Pastel", "柔和粉彩", "柔和粉彩", "ソフトパステル"),
    ("Classic B&W", "經典黑白", "经典黑白", "クラシック白黒"),
    ("High Contrast B&W", "高對比黑白", "高对比黑白", "ハイコントラスト白黒"),
    ("Selenium Tone", "硒色調", "硒色调", "セレニウム調色"),
    ("Sepia", "復古棕褐", "复古棕褐", "セピア"),
    ("Infrared", "紅外線", "红外线", "赤外線"),
    ("Faded Matte", "霧面褪色", "雾面褪色", "フェードマット"),
    ("Portra Film", "Portra 底片", "Portra 胶片", "Portra フィルム"),
    ("Kodachrome", "柯達康", "柯达康", "コダクローム"),
    ("Vintage", "復古", "复古", "ヴィンテージ"),
    ("Cross Process", "交叉沖印", "交叉冲印", "クロスプロセス"),
    ("Crisp Landscape", "清晰風景", "清晰风景", "くっきり風景"),
    ("Moody Forest", "幽暗森林", "幽暗森林", "ムーディーな森"),
    ("Blue Hour", "藍調時刻", "蓝调时刻", "ブルーアワー"),
    ("Soft Portrait", "柔美人像", "柔美人像", "ソフトポートレート"),
    ("Warm Skin", "溫暖膚色", "温暖肤色", "暖かい肌色"),
    ("Moody", "情緒", "情绪", "ムーディー"),
    ("Cinematic", "電影感", "电影感", "シネマティック"),
    ("Bright & Airy", "明亮清透", "明亮清透", "明るく軽やか"),
    ("My presets", "我的預設集", "我的预设", "マイプリセット"),
    ("Save the current settings as a preset to reuse them.", "將目前設定儲存為預設集以便重複使用。", "将当前设置保存为预设以便重复使用。", "現在の設定をプリセットとして保存して再利用できます。"),
    ("Delete preset", "刪除預設集", "删除预设", "プリセットを削除"),
    ("Preset name", "預設集名稱", "预设名称", "プリセット名"),
    ("Save", "儲存", "保存", "保存"),
    ("Saved preset {}", "已儲存預設集 {}", "已保存预设 {}", "プリセット {} を保存しました"),
    // Crop and masks
    ("Aspect", "長寬比", "长宽比", "縦横比"),
    ("Free", "自由", "自由", "自由"),
    ("Swap portrait and landscape", "切換直式與橫式", "切换竖向与横向", "縦向きと横向きを切り替え"),
    ("Overlay", "參考線", "参考线", "オーバーレイ"),
    ("Rule of thirds", "三分法", "三分法", "三分割法"),
    ("Golden ratio", "黃金比例", "黄金比例", "黄金比"),
    ("Grid", "格線", "网格", "グリッド"),
    ("Diagonals", "對角線", "对角线", "対角線"),
    ("Reset crop", "重設裁切", "重置裁剪", "切り抜きをリセット"),
    (
        "Drag the corners or edges to crop, and inside the frame to move it. Double-click or press Enter to finish.",
        "拖曳角落或邊緣進行裁切，拖曳框內可移動。按兩下或按 Enter 完成。",
        "拖动角或边进行裁剪，拖动框内可移动。双击或按 Enter 完成。",
        "角や辺をドラッグして切り抜き、枠の内側をドラッグして移動します。ダブルクリックか Enter で完了します。",
    ),
    ("+ Linear gradient", "＋ 線性漸層", "＋ 线性渐变", "＋ 線形グラデーション"),
    ("+ Radial gradient", "＋ 放射狀漸層", "＋ 径向渐变", "＋ 円形グラデーション"),
    ("Linear {}", "線性 {}", "线性 {}", "線形 {}"),
    ("Radial {}", "放射 {}", "径向 {}", "円形 {}"),
    (
        "Add a gradient, then drag its handles on the photo and set what it changes.",
        "新增漸層後，在相片上拖曳控制點並設定要調整的項目。",
        "添加渐变后，在照片上拖动控制点并设置要调整的项目。",
        "グラデーションを追加し、写真上のハンドルをドラッグして補正内容を設定します。",
    ),
    ("Show the selected mask in red", "以紅色顯示選取的遮罩", "以红色显示选中的蒙版", "選択したマスクを赤で表示"),
    ("Delete mask", "刪除遮罩", "删除蒙版", "マスクを削除"),
    ("Invert", "反轉", "反相", "反転"),
    // Export options
    ("Full size", "原始大小", "原始尺寸", "フルサイズ"),
    ("Long edge of the exported file", "匯出檔案的長邊", "导出文件的长边", "書き出すファイルの長辺"),
    ("Quality", "品質", "质量", "画質"),
    // EXIF and video details
    ("Lens", "鏡頭", "镜头", "レンズ"),
    ("Exposure time", "曝光時間", "曝光时间", "露出時間"),
    ("Aperture", "光圈", "光圈", "絞り値"),
    ("Focal length", "焦距", "焦距", "焦点距離"),
    ("35 mm equiv.", "35mm 等效焦距", "35mm 等效焦距", "35mm 換算"),
    ("Exposure bias", "曝光補償", "曝光补偿", "露出補正"),
    ("Program", "曝光程式", "曝光程序", "露出プログラム"),
    ("Metering", "測光模式", "测光模式", "測光モード"),
    ("Flash", "閃光燈", "闪光灯", "フラッシュ"),
    ("White balance", "白平衡", "白平衡", "ホワイトバランス"),
    ("Color space", "色彩空間", "色彩空间", "色空間"),
    ("Altitude", "海拔", "海拔", "高度"),
    ("Software", "軟體", "软件", "ソフトウェア"),
    ("Artist", "作者", "作者", "作成者"),
    ("Copyright", "版權", "版权", "著作権"),
    ("Fired", "已閃光", "已闪光", "発光"),
    ("Did not fire", "未閃光", "未闪光", "発光なし"),
    ("Audio", "音訊", "音频", "音声"),
    ("None", "無", "无", "なし"),
    ("Bit rate", "位元速率", "比特率", "ビットレート"),
    ("Unknown date", "日期不明", "日期未知", "日付不明"),
    // Errors
    ("Install FFmpeg to read and play videos", "請安裝 FFmpeg 以讀取和播放影片", "请安装 FFmpeg 以读取和播放视频", "動画を再生するには FFmpeg をインストールしてください"),
    ("FFmpeg could not read this video", "FFmpeg 無法讀取此影片", "FFmpeg 无法读取此视频", "FFmpeg でこの動画を読み込めませんでした"),
    ("The file has no video track", "此檔案沒有影片軌", "此文件没有视频轨道", "このファイルには映像トラックがありません"),
    ("FFmpeg is not installed", "尚未安裝 FFmpeg", "未安装 FFmpeg", "FFmpeg がインストールされていません"),
    ("No frame could be decoded", "無法解碼任何畫面", "无法解码任何画面", "フレームをデコードできませんでした"),
    ("Can't start FFmpeg: {}", "無法啟動 FFmpeg：{}", "无法启动 FFmpeg：{}", "FFmpeg を起動できません: {}"),
    ("Can't open folder: {}", "無法開啟資料夾：{}", "无法打开文件夹：{}", "フォルダを開けません: {}"),
    ("That path is not a folder", "這個路徑不是資料夾", "该路径不是文件夹", "このパスはフォルダではありません"),
    ("Album name is empty", "相簿名稱不可空白", "相册名称不能为空", "アルバム名を入力してください"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_system_locales() {
        assert_eq!(Lang::from_locale("zh-TW"), Some(Lang::TraditionalChinese));
        assert_eq!(Lang::from_locale("zh-Hant-HK"), Some(Lang::TraditionalChinese));
        assert_eq!(Lang::from_locale("zh_CN.UTF-8"), Some(Lang::SimplifiedChinese));
        assert_eq!(Lang::from_locale("zh-Hans-SG"), Some(Lang::SimplifiedChinese));
        assert_eq!(Lang::from_locale("ja-JP"), Some(Lang::Japanese));
        assert_eq!(Lang::from_locale("en-US"), Some(Lang::English));
        assert_eq!(Lang::from_locale("fr-FR"), None);
        assert_eq!(Lang::from_code("zh-Hant"), Some(Lang::TraditionalChinese));
    }

    #[test]
    fn table_rows_are_unique_and_keep_placeholders() {
        let mut seen = std::collections::HashSet::new();
        for (en, zh_hant, zh_hans, ja) in TABLE {
            assert!(seen.insert(*en), "duplicate entry {en:?}");
            let slots = en.matches("{}").count();
            for text in [zh_hant, zh_hans, ja] {
                assert_eq!(text.matches("{}").count(), slots, "placeholders differ in {text:?}");
            }
        }
    }

    #[test]
    fn fills_placeholders_in_order() {
        assert_eq!(trf("Can't watch {}: {}", &[&"D:\\Photos", &"denied"]), "Can't watch D:\\Photos: denied");
        assert_eq!(trf("{} photos", &[&3]), "3 photos");
    }
}
