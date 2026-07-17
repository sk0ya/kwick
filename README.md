# Kwick

Rust製の軽量コマンドランチャー(Windows専用)。
Keypirinha的な拡張性(Luaプラグイン)と、ueli的な設定のしやすさ(TOML)の両立を目指す。

## ビルドと起動

```
cargo build --release
.\target\release\kwick.exe
```

起動するとウィンドウが表示され、以降はホットキー(既定: `Alt+Space`)でトグルします。
フォーカスを失うと自動的に隠れます。タスクトレイに常駐し、トレイアイコンの
左クリックでも表示/非表示を切り替えられます(右クリックでメニュー)。
終了はトレイメニューの「終了」か、`Kwick: Quit` を検索して実行。

- `kwick.exe --hidden` でウィンドウを出さずに常駐開始
- `Kwick: Register Startup` を実行するとWindowsログオン時に自動起動(`--hidden`付き)
- 二重起動は単一インスタンスガードで防止されます

## 使い方

- 文字を入力するとスタートメニューのアプリ・PATH上の実行ファイルをファジー検索
- `↑` `↓` で選択、`Enter` で実行、`Esc` で閉じる
- 空欄のときは使用頻度の高いアイテムを表示(起動回数は `history.toml` に記録され、
  検索順位のブーストにも使われる)
- `g rust` のように「キーワード + 検索語」でWeb検索(設定で追加可能)
- `1+2*3` のように数式を入力すると電卓(同梱Luaプラグイン)

## 設定

`%APPDATA%\kwick\config.toml` — ウィンドウを表示するたびに再読み込みされるので、編集して即反映。

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
```

> ⚠ `alt+space` はPowerToys Runなど他のランチャーと競合しがち。設定したキーが
> 使えない場合は `ctrl+alt+space` → `ctrl+shift+space` → `ctrl+alt+k` の順に
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
                -- アクションは以下のいずれか1つ:
                cmd = "notepad", args = "C:\\memo.txt",  -- ShellExecute
                -- url = "https://example.com",           -- ブラウザで開く
                -- run = function() kwick.copy("text") end, -- 任意のLua処理
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

サンプルとして `calc.lua`(電卓)が初回起動時に生成されます。

## アーキテクチャ

```
src/
  main.rs        エントリポイント・単一インスタンスガード・ウィンドウ設定
  app.rs         eframe App(UI・キーハンドリング)
  winctl.rs      Win32 ShowWindow による表示/非表示制御
  tray.rs        タスクトレイアイコンとメニュー
  config.rs      TOML設定の読み込み・初期ファイル生成
  history.rs     起動回数の記録(頻度ブースト・空欄時の表示)
  matcher.rs     ファジーマッチ(nucleo-matcher)
  lua_host.rs    Luaプラグインホスト(mlua)
  launch.rs      ShellExecuteW ラッパ・スタートアップ登録
  fonts.rs       日本語フォントのフォールバック読み込み
  providers/
    apps.rs      スタートメニュー(.lnk/.url)スキャン
    pathbin.rs   PATH上の実行ファイルスキャン
    mod.rs       Item/Action定義・設定由来アイテム・ビルトイン
```

- 重いスキャン(アプリ・PATH)は起動時のみ。`Kwick: Reload Index` かトレイメニューで再スキャン。
- 設定とLuaプラグインはウィンドウ表示のたびにリロード(ホットリロード)。
- **表示/非表示はegui経由ではなくWin32 `ShowWindow`を直接呼ぶ**(`winctl.rs`)。
  非表示中はWindowsが`WM_PAINT`を配送せず`update()`が走らないため、
  ホットキー/トレイのスレッドからeguiのViewportCommandを送っても処理されない。
  この制約を回避するための設計なので、可視性制御をeguiに戻さないこと。

## ロードマップ(未実装)

- [ ] アプリアイコン表示(.lnkからのアイコン抽出)
- [ ] UWPアプリの列挙
- [ ] Everything SDK連携によるファイル検索
- [ ] 二重起動時に既存インスタンスへ「表示」を通知する(現状はメッセージボックス)
