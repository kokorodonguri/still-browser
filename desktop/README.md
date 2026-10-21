# Still for Windows（Rust + Tauri / WebView2）— 0.4.0 まで

0.5.0 から Windows 版は Firefox ESR を同梱した版（[windows/README.md](../windows/README.md)）に置き換えました。このフォルダーはそれ以前の版のソースで、もう配布していません。インストール済みの 0.4.0 は、この版の自動更新（`update/windows.json`）で Firefox 版に移ります。以下は 0.4.0 時点の説明です。

### インストール

| ファイル | 内容 |
|---|---|
| `artifacts/Still_0.3.0_x64-setup.exe` / `Still_0.3.0_arm64-setup.exe` | インストーラー（約1.5MB）。管理者権限は不要です。WebView2 Runtime が入っていなければ自動で取得します |
| `artifacts/StillBrowser-Windows-x64.exe` / `StillBrowser-Windows-arm64.exe` | インストール不要の単体版（約4MB）。WebView2 Runtime が必要です。アップデートするとインストーラーが実行され、以後はインストール版になります |

どれもコード署名はしていません。初回起動時に SmartScreen の警告が出ることがあります。

### 使い方

画面はすべて HTML/CSS で描画しています。スクロールバー、メニュー、タイトルバーなどに Windows 標準の部品は使っていません。

- 左上のボタンでサイドバーを開閉します。サイドバー上部の切り替えで、タブ・ブックマーク・履歴・ダウンロードを表示します。
- サイドバー開閉ボタンの右の歯車ボタン（または `Ctrl+,`）で設定画面を開きます。`Esc` か、タブを選ぶと閉じます。
  - General：起動時に開くもの（前回のタブ／新しいタブ／ホームページ）、ホームページ、ダウンロード先フォルダ、毎回保存先を確認するか、アップデートの確認
  - Appearance：テーマ（自動／ライト／ダーク。ページの配色にも反映）、アクセントカラー、新しいタブの背景画像（PNG／JPEG／GIF／WebP／AVIF／BMP、20MBまで）、起動時のサイドバー
  - Search：検索エンジン（Google／Bing／DuckDuckGo／Yahoo! JAPAN）
  - Privacy：閲覧履歴を残すか、閲覧データ（履歴・Cookie とサイトデータ・キャッシュ）の削除
  - Passwords：取り込んだパスワードの一覧（サイトとユーザー名だけを表示）と削除
  - Import：この PC にある他のブラウザ（プロファイルごと）からブックマーク・履歴・Firefox の Cookie を取り込みます。ブックマーク（HTML）・パスワード（CSV）・Google Takeout の履歴（JSON）のファイルも取り込めます。相手のブラウザを開いたままでも、データベースを複製してから読みます
  - Shortcuts：キー操作の一覧
- キー操作：`Ctrl+L`（アドレス欄）、`Ctrl+T`（新規タブ）、`Ctrl+W`（タブを閉じる）、`Ctrl+Tab` / `Ctrl+Shift+Tab`（タブ切り替え）、`Ctrl+,`（設定）
- 二重に起動すると、既に開いているウィンドウが前面に出ます。`still-browser.exe https://example.org` のように URL を渡すと、新しいタブで開きます。
- 大きさを指定したログイン用ポップアップは、別ウィンドウで開きます。このウィンドウだけは Windows 標準の枠が付きます。
- ページが止まったりクラッシュしたりした場合は、通知が出ます。再読み込みで復帰できます。
- ウィンドウの位置と大きさは、次回起動時に復元します。
- 開いているサイトのパスワードを保存している場合は、アドレス欄に鍵ボタンが出ます。押すとユーザー名とパスワードを入力します（複数あるときはアドレス欄の位置で選びます）。ページに入力したパスワードは、WebView2 の機能で保存し、次回から自動で入力することもできます。

### データの保存先（`%LOCALAPPDATA%\StillBrowser\`）

| パス | 中身 |
|---|---|
| `state.json` | タブ・ブックマーク・履歴・ダウンロード・ウィンドウ位置・設定。C# 版のデータもそのまま読めます |
| `passwords.dat` | 取り込んだパスワード。Windows のユーザーごとに DPAPI で暗号化しています |
| `WebView2\` | Cookie・ログイン情報 |
| `NewTab\` | 新しいタブの背景画像（選んだファイルのコピー） |
| `UI\` | 画面用 |
| `logs\still.log` | ログ（1MB で `still.1.log` に切り替え） |

壊れたファイルや、新しいバージョンが書いたファイルは、上書きする前にバックアップします。

設計（要件、エッジケース、データの流れ、保存形式、IPC）は [DESIGN.md](DESIGN.md) にまとめています。

### ビルド（Linux からのクロスビルド）

```sh
rustup target add aarch64-pc-windows-msvc x86_64-pc-windows-msvc
cargo install cargo-xwin                 # clang / lld-link / llvm-rc / makensis が PATH に必要
(cd desktop/ui && npm ci)
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/still-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
cd desktop && ui/node_modules/.bin/tauri build --runner cargo-xwin --target aarch64-pc-windows-msvc
```

署名用の環境変数がないとビルドは失敗します（アップデートの署名 `.exe.sig` を作るため）。

成果物は `target/<target>/release/` に出ます。単体版の EXE と、インストーラーとその署名（`bundle/nsis/`）です。

この作業環境では、LLVM 18 と NSIS を `~/.local/llvm18` と `~/.local/nsis` に root 権限なしで展開しています。

Windows 上でのビルドとテストは `.github/workflows/desktop.yml`（GitHub Actions）で実行します。

### テスト

- `cargo test -p still-core`（desktop/ 内で実行）：タブ・保存・クラッシュ復帰などの状態遷移
- `cargo xwin clippy -p still-browser --target <target> -- -D warnings`：Windows 専用コードのチェック
- `.venv-linux/bin/python desktop/ui/tests/ui_check.py`：画面を Firefox で確認

Windows 実機での動作確認はまだしていません。
