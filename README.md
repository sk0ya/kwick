# Kwick

Rust製の軽量コマンドランチャー(Windows専用)。
Keypirinha的な拡張性(Luaプラグイン)と、ueli的な設定のしやすさ(TOML)の両立を目指す。

## ビルドと起動

```
cargo build --release
.\target\release\kwick.exe
```

起動するとウィンドウが表示され、以降はホットキー(既定: `Alt+Space`)で表示・前面化します。
表示中にもう一度押しても閉じません。閉じるには `Esc` を押します。
フォーカスを失うと自動的に隠れます。タスクトレイに常駐し、トレイアイコンの
左クリックでも表示/非表示を切り替えられます(右クリックでメニュー)。
終了はトレイメニューの「終了」か、`Kwick: Quit` を検索して実行。

- `kwick.exe --hidden` でウィンドウを出さずに常駐開始
- `Kwick: Register Startup` を実行するとWindowsログオン時に自動起動(`--hidden`付き)
- 起動中にもう一度 `kwick.exe` を実行すると、常駐中のウィンドウが表示されます
- 環境変数 `KWICK_CONFIG_DIR` を指定すると、別の設定フォルダを使う別インスタンスとして起動します(開発用)

## 使い方

- 文字を入力すると登録済みアプリ・Microsoft Store アプリ・スタートメニューのアプリなどをファジー検索
- `↑` `↓` で選択、`Enter` で実行、`Esc` で閉じる
- 空欄のときは固定したアイテムと使用頻度の高いアイテムを表示(起動回数は `history.toml`)
- 一度選んだアイテムは「そのとき打った文字列」と一緒に学習され(`learned.toml`)、
  次に同じ文字列(またはその先頭部分)を打つと最上位に出ます
- `g rust` のように「キーワード + 検索語」でWeb検索(設定で追加可能)
- `1+2*3` のように数式を入力すると電卓(同梱Luaプラグイン)

### キー操作

| キー | 動作 |
|---|---|
| `Enter` | 実行 |
| `Ctrl+K` / 右クリック | アクションパネル(選択中アイテムにできる操作の一覧) |
| `Ctrl+Shift+Enter` | 管理者として実行 |
| `Shift+Enter` | 引数を入力してから実行 |
| `Ctrl+Enter` | ファイルの場所を開く(ショートカットはリンク先) |
| `Ctrl+C` | パスをコピー(検索欄で文字を選択していないとき) |
| `Tab` | フォルダの中へ / モードに入る |
| `Ctrl+P` | プレビューペインの表示切り替え |
| `Del` | 空欄時の一覧で履歴から削除 |
| `Esc` | 閉じる(サブリスト・アクションパネル・引数入力中は一つ戻る) |

アクションパネルからはほかに「上位に固定」「候補から隠す」「履歴から削除」ができます
(固定・非表示は `config.toml` の `pinned` / `hidden` に保存)。

### 検索モードと即答

| 入力例 | 動作 |
|---|---|
| `w chrome` | 開いているウィンドウに切り替え(閉じる・プロセス終了もアクションパネルから) |
| `f report` | ファイル検索([Everything](https://www.voidtools.com/) が起動している必要あり) |
| `cb ` | クリップボード履歴。`Enter` で直前のウィンドウに貼り付け |
| `kill chrome` | プロセスを終了(同名プロセスすべて) |
| `:ねこ` `:cat` | 絵文字を検索して貼り付け |
| `C:\Us` `~\Down` `%APPDATA%\` | パスを入力するとフォルダの中身を表示。`Tab` で中へ |
| `10km to mi` `100 c in f` `20坪 to m2` | 単位変換(長さ・重さ・体積・面積・温度・データ量・時間・速度) |
| `100 usd in jpy` | 為替換算(初回のみ open.er-api.com からレートを取得、12時間キャッシュ) |
| `now` `today+30d` `2026-12-25` `unix 1700000000` | 日付計算(`Enter` でコピー) |
| `vol 30` | 音量を 30% に(ほかに「音量ミュート切り替え」「ゴミ箱を空にする」など) |
| `ip` | IP アドレスの一覧(`Enter` でコピー) |
| `? 質問` | Claude に質問(同梱 `claude.lua`、API キーの設定が必要) |

モードのプレフィックスは `[prefixes]` で変更でき、空文字にするとそのモードは無効になります。
「ウィンドウ切り替え」「クリップボード履歴」などモード名で検索して `Tab`/`Enter` でも入れます。

クリップボード履歴はテキストのみ・メモリ上だけに保持します(ディスクには書きません)。
パスワードマネージャーが「履歴に残さない」印を付けたコピーは記録しません。
不要なら `clipboard_history = false`。

## 設定

`%APPDATA%\kwick\config.toml` — ウィンドウを表示するたびに再読み込みされるので、編集して即反映。

開き方はどちらでも:

- トレイアイコン右クリック →「設定を開く」
- ランチャーで `settings`(または `config`)と入力 → `Kwick: Settings` を実行

いずれも `config.toml` を関連付けられたエディタで開きます(関連付けがなければメモ帳)。

```toml
hotkey = "alt+space"     # 例: "ctrl+alt+k", "win+space"
max_results = 8

# コード不要のカスタムコマンド
[[commands]]
name = "Shutdown PC"
cmd = "shutdown"
args = "/s /t 0"
keyword = "sd"           # 別名(検索にヒットする)

# Web検索
[[web_searches]]
name = "Google"
keyword = "g"
url = "https://www.google.com/search?q={query}"

# {query} を含まなければクイックリンク(名前かキーワードで開く)
[[web_searches]]
name = "作業フォルダ"
keyword = "wk"
url = 'D:\Work'
```

カスタムコマンドとクイックリンクは、キーワードを完全一致で打つと最上位に出ます。
カスタムコマンドに `instant = true` を付けると、キーワードを打ち終えた時点で
`Enter` なしで実行されます(誤爆しやすいので、安全なコマンドだけに付けてください)。

`scan_registered_apps = true` (既定) にすると、Windows の App Paths に登録された
起動可能なアプリも検索対象になります。PATH 全体を検索するより候補が少なく、GUI アプリを
増やしたい場合に向いています。

> ⚠ `alt+space` はPowerToys Runなど他のランチャーと競合しがち。設定したキーが
> Spaceを使う設定では、OS登録に失敗してもキーボードフックと独立したキー状態監視で
> 設定したキーを維持します。競合するアプリも反応する場合は、そのアプリの設定を変更してください。
> それ以外のキーが使えない場合は `ctrl+alt+space` → `ctrl+shift+space` → `ctrl+alt+k` の順に
> 自動フォールバックし、実際に使われているキーがウィンドウ下部とトレイアイコンの
> ツールチップに表示されます。

## Luaプラグイン

`%APPDATA%\kwick\plugins\*.lua` に置くと読み込まれます(表示のたびにリロード)。

```lua
kwick.register{
    name = "myplugin",
    -- 入力が変わるたびに呼ばれる。マッチしないときは {} を返す。
    on_query = function(query)
        return {
            {
                title = "表示されるタイトル",
                subtitle = "説明",
                icon = "C:\\Windows\\notepad.exe",       -- 省略可: このファイルのアイコンを表示
                -- アクションは以下のいずれか1つ:
                cmd = "notepad", args = "C:\\memo.txt",  -- ShellExecute
                -- url = "https://example.com",           -- ブラウザで開く
                -- copy = "text",                         -- クリップボードにコピー
                -- run = function() ... end,              -- 任意のLua処理(ウィンドウは閉じる)
                -- run = function() ... end, keep_open = true, -- 閉じずに実行(結果を後で更新する用)
                -- items = function() return { ... } end, -- Enter でサブリストを開く(表でも可)

                -- アクションパネル(Ctrl+K)に並ぶ追加の操作
                actions = {
                    { title = "コピー", copy = "C:\\memo.txt" },
                },
            },
        }
    end,
}
```

提供API:

| 関数 | 説明 |
|---|---|
| `kwick.register(table)` | プラグインを登録する |
| `kwick.copy(text)` | クリップボードにコピーする |
| `kwick.open(target, args?)` | ファイル・URL を開く |
| `kwick.http(opts, callback)` | 非同期 HTTP。`opts` は URL 文字列か `{url, method, headers, body}`。完了後 `callback({ok, status, body, error})` が UI スレッドで呼ばれる |
| `kwick.refresh()` | 検索結果を作り直す(非同期の結果をキャッシュした後に呼ぶ) |
| `kwick.exec(cmd)` | コマンドを実行して `stdout, 終了コード` を返す(同期。短い処理だけに) |
| `kwick.notify(title, text)` | デスクトップ通知 |
| `kwick.settings(name)` | `config.toml` の `[plugins.<name>]` を表で返す |
| `kwick.json_decode(text)` / `kwick.json_encode(value)` | JSON 変換(decode は失敗時 `nil, エラー`) |

サンプルとして `calc.lua`(電卓)と `claude.lua`(Claude に質問)が初回起動時に生成されます。
`claude.lua` は API キーを設定するまで何もしません:

```toml
[plugins.claude]
api_key = "sk-ant-..."        # または環境変数 ANTHROPIC_API_KEY
model = "claude-opus-5-5"     # 省略可
effort = "low"                # 省略可 (low / medium / high)
```

質問は `Enter` を押したときだけ送信され、回答はその場に表示されます(`Enter` でコピー)。

## アーキテクチャ

```
src/
  main.rs        エントリポイント・ウィンドウ設定
  instance.rs    単一インスタンスガード・再起動時の表示通知
  app.rs         eframe App(UI・キーハンドリング・検索モードの振り分け)
  winctl.rs      Win32 ShowWindow による表示/非表示制御
  tray.rs        タスクトレイアイコンとメニュー
  config.rs      TOML設定の読み込み・初期ファイル生成
  history.rs     起動回数とクエリ学習の記録
  matcher.rs     ファジーマッチ(nucleo-matcher)
  lua_host.rs    Luaプラグインホスト(mlua)
  json.rs        Lua 用の小さな JSON 変換
  http.rs        WinHTTP による HTTP クライアント(TLS 実装を同梱しない)
  clipboard.rs   クリップボード操作・履歴(リスナー方式)・貼り付け
  everything.rs  Everything の IPC クライアント(WM_COPYDATA)
  preview.rs     プレビューペインの内容
  notify.rs      デスクトップ通知
  launch.rs      ShellExecuteW ラッパ・管理者実行・ファイルの場所を開く
  icons.rs       シェルアイコン・サムネイル(ワーカースレッド)
  fonts.rs       日本語フォントのフォールバック読み込み
  providers/
    mod.rs       Item/Action定義・アクションパネル・設定由来アイテム
    apps.rs      スタートメニュー(.lnk/.url)スキャン
    registered.rs Windows の App Paths に登録されたアプリのスキャン
    uwp.rs       Microsoft Store アプリ(shell:AppsFolder)
    pathbin.rs   PATH上の実行ファイルスキャン
    winlist.rs   ウィンドウ一覧・プロセス一覧
    pathnav.rs   パス入力によるフォルダ閲覧
    convert.rs   単位・為替・日付の即答
    sysops.rs    音量・ゴミ箱・IP アドレス
    emoji.rs     絵文字(assets/emoji.tsv、初回使用時に読み込み)
```

- 重いスキャン(アプリ・PATH)は起動時・検索対象設定の変更時にバックグラウンドで実行する。`Kwick: Reload Index` かトレイメニューでも再スキャン可能。
- 設定とLuaプラグインはファイルの変更を検知したときだけリロード(ホットリロード)。
- インデックスの再スキャンはバックグラウンドで行い、完了するまで現在の候補を使い続ける。
- **表示/非表示はegui経由ではなくWin32 `ShowWindow`を直接呼ぶ**(`winctl.rs`)。
  非表示中はWindowsが`WM_PAINT`を配送せず`update()`が走らないため、
  ホットキー/トレイのスレッドからeguiのViewportCommandを送っても処理されない。
  この制約を回避するための設計なので、可視性制御をeguiに戻さないこと。

- 常駐用に追加したスレッドはイベント待ちだけ(クリップボードの変更通知、再起動時の表示通知)で、
  ポーリングはしない。Everything・為替・Lua の HTTP は使ったときだけ動く。
- `assets/emoji.tsv` は Unicode の emoji-test.txt と CLDR アノテーション(ja/en)から生成
  (Unicode License v3)。
