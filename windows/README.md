# Still for Windows

Mozilla の Firefox ESR に、Still 独自の縦タブ・配色・新しいタブ画面を備えた Windows 版です。無機質な金属をテーマにしたフラットな配色（ダークはガンメタル、ライトはスチールグレー）で、フレーム・検索・設定のデザインをそろえています。Firefox は Mozilla 配布のものをそのまま使うか、メモリ使用量を減らした Still 独自のビルド（`gecko/`）を使います。どちらにも、Linux 版と共通の CSS・初期設定（`linux/profile/`）に、Windows 用の色定義と `profile/userChrome.css` を重ねます。設計と実装範囲は [DESIGN.md](DESIGN.md) を参照してください。

- 画面の構成は以前の Still（Tauri 版）と同じです。ツールバーは高さ 40px の1行：左にサイドバー（塗りの角丸）・戻る・進む・再読み込み、続いて塗りのアドレス欄（編集中以外はアドレスを中央に表示）、右にダウンロード・新しいタブ。サイトの識別情報とトラッキング防止の表示は Firefox のものを使います。右端の … からメニューを開きます。
- ページは半径 4px の角の面です。新規プロファイルは幅 200px の縦タブを開いた状態で始まり、開閉できます。Firefox 153 では幅のドラッグ調整も維持します。既存プロファイルの開閉状態と幅は変えません。サイドバーの上に「タブ・ブックマーク・履歴・ダウンロード」の切り替え、その下に「タブ」の見出しとタブの一覧が並びます。ブックマークと履歴は Firefox のサイドバーパネルを開き、タブで閉じ、ダウンロードは about:downloads を開きます。各タブはサイトのアイコンと、タイトルの下にホスト名を表示します。設定ボタンはタブ欄の一番下に固定し、折りたたんでも歯車アイコンから開けます。
- ライトは明るいスチールグレー、ダークはガンメタル。どちらも青みのある灰色の単色の面で、グラデーション・光沢・影は使いません。主操作とフォーカスはスチールブルーグレーです。ブラウザ・新しいタブ・設定で配色を共有します（`firefox/still/content/tokens.css`）。
- 新しいタブは Still 専用の検索画面です。既定の検索エンジンで検索し、ブックマーク・履歴・ダウンロードを開けます。新規プロファイルではホームページにも設定されます。
- テーマは新規プロファイルではライト。Still の設定で Windows に合わせる／ライト／ダークを選べます。既存プロファイルに保存済みのテーマは維持します（Linux 版は別の初期設定です）。
- 設定ボタンと ≡ メニューの「設定」は、Still の設定画面を開きます。左に項目、右に内容が並び、狭いウィンドウでは1列になります。
- ウィンドウのアイコンとタイトルも Still のものです。プライベートウィンドウには「Still プライベートブラウジング」と表示します。タスクバーとスタートメニューのショートカットは `still.exe` のアイコンを使います。

| 項目 | 内容 |
|---|---|
| 対応 OS | Windows 10 / 11（x64、ARM64） |
| 配布ファイル | `artifacts/Still_<版>_x64-setup.exe` / `Still_<版>_arm64-setup.exe`（約80〜100MB）と、それぞれの更新署名 `.exe.sig`、署名とインストーラーを結び付ける記録 `.exe.release.json`（最後に置かれ、`site/build.py` はこれと一致するインストーラーだけを公開します） |
| インストール先 | `%LOCALAPPDATA%\Still`（管理者権限は不要）。以前の Tauri 版と同じ場所に上書きします |
| データ | `%LOCALAPPDATA%\StillBrowser\Profile`（Still 専用の Firefox プロファイル。通常の Firefox とは別） |

## 中身

| パス | 役割 |
|---|---|
| `launcher/` | `still.exe`（Rust）。プロファイルを準備して、同梱の Firefox を `-profile` 付きで起動します。Still 自体の更新も扱います |
| `profile/userChrome.css` | Still のフレーム・縦タブ・ワードマーク。Firefox 140 と 153 のサイドバー要素名に対応します |
| `profile/userContent.css` | Windows 用の組み込みページの配色。通常のウェブサイトは対象外です |
| `firefox/still/content/tokens.css` | ブラウザと設定画面で共通の配色・角丸・余白。`still.exe` が共通 CSS と Windows 用 CSS の間に埋め込みます |
| `firefox/still.cfg`、`firefox/defaults/pref/still-autoconfig.js` | Firefox の AutoConfig。起動のたびに設定画面を登録し、各ウィンドウのタイトルと設定ボタンを整えます。ツールバーの並びと以前の版のブックマーク・履歴の取り込みは、プロファイルごとに一度だけ適用します |
| `firefox/still/` | 権限のある設定画面 `settings.html`、新しいタブ `newtab.html` と、その CSS・JS。インストール先の `firefox/still/` に追加します |
| `firefox/browser/chrome/icons/default/`（配布時に生成） | `package.py` が Still の `icon.ico` を `main-window.ico` と `default.ico` として追加します。Firefox 本体のアイコンリソースは変更しません |
| `installer.nsi` | NSIS のインストーラー |
| `package.py` | Firefox の取得からインストーラーの署名までを行うビルドスクリプト |

## 使い方

- スタートメニューの「Still」から起動します。2回目以降に起動すると、開いているウィンドウに新しいタブで開きます。
- タブ欄の一番下の設定ボタンか ≡ メニューの「設定」で、Still の設定を新しいタブに開きます。既に開いていればそのタブに切り替えます（通常／プライベートのウィンドウは分けて扱います）。アドレス欄に `chrome://still/content/settings.html` と入力しても開けます。
- 設定には「一般・外観・検索・プライバシー・引き継ぎ・情報」があります。起動時のタブとホームページ、テーマ、検索エンジンと検索候補、履歴を変更できます。変更はそのまま保存され、画面を閉じて開き直すと Firefox の設定から読み直します。閲覧データの削除とパスワードの管理は Firefox の画面を使います。
- 設定画面の「設定を検索」で、項目を絞り込めます。「壁紙」「お気に入り」や英語名でも探せます。検索欄で Esc、「クリア」、項目メニューの選択で検索を解除できます。検索語は保存しません。
- アドレス欄に入力すると、履歴・ブックマーク・開いているタブと、検索エンジン（初期状態では Google）の候補を出します。検索候補を止めるには、Still の設定 › 検索 › 「検索候補を表示する」を外します。
- 他のブラウザ（Chrome・Edge など）からの引き継ぎは、Still の設定 › 引き継ぎ › 「引き継ぐ…」から Firefox の取り込み機能を開きます。`still.exe --import` と `about:preferences#general-migrate` も引き続き使えます。
- 新しいタブの背景画像は、Still の設定 › 外観 › 「選ぶ…」で選び、「外す」で解除します。PNG・JPEG・GIF・WebP・AVIF・BMP（20MBまで）を使えます。`still.exe --newtab-image`（`--newtab-image 画像のパス` も可）と `still.exe --remove-newtab-image` と同じ処理で、Firefox は画像の CSS を起動時にしか読まないため、再起動後に反映されます。
- 既定のブラウザにするには、Still の設定 › 一般 › 「既定のブラウザにする…」（`still.exe --make-default`）を押し、開いた Windows の設定（既定のアプリ）で Still を選びます。Windows はアプリが自分で既定を変えることを許していないため、最後の選択はユーザーが行います。インストーラーが Still をブラウザとして登録し（`HKCU\Software\Clients\StartMenuInternet\Still`、http・https と .html などのファイル）、アンインストールで登録を外します。Firefox 本体の「既定のブラウザにする」は Still のプロファイルを使わない登録になるため、無効にしています。
- 情報には Still と Firefox の版、ライセンス表記、「アップデートを確認」があります。一番下の「Firefox の詳細設定を開く」から元の設定画面を開けます。`about:preferences` を直接入力しても Firefox の設定画面です。
- 初回起動時に Firefox の利用規約の確認が出ます。「続行」を押すと以後は出ません。

## 以前の Windows 版（Tauri / WebView2）からの移行

- 以前の版の自動更新がこのインストーラーを `/P /UPDATE` で実行し、同じフォルダーに上書きします。スタートメニューとアンインストールの登録も置き換えます。
- 最初の起動時に、`%LOCALAPPDATA%\StillBrowser\state.json` のブックマーク（Firefox の「ブックマークメニュー」へ）と履歴、新しいタブの画像を引き継ぎます。取り込みは一度だけです（プロファイルの `still-import.done` が目印）。
- 保存したパスワードとログイン状態（Cookie）は引き継げません。サイトにはログインし直してください。以前のデータ（`state.json`、`WebView2\` など）は消さずに残します。

## 更新

- 0.5.0 で作ったプロファイルは、移行リビジョン 2 の最初の起動でダーク固定を外します（`still.windowsRevision`）。現行版の移行初期値はライトです。
- `still.exe` は起動するたびに `https://still.dongurihub.com/update/windows.json` を確認します。新しい版があると確認のダイアログを出し、「はい」でダウンロードして、以前の版と同じ鍵（`plugins.updater.pubkey`）で署名を確かめます。インストールは次に Still を起動したとき、Firefox が動いていない状態で行います（`%LOCALAPPDATA%\StillBrowser\Update`）。「いいえ」を選んだ版は、次の版が出るまで聞きません。`still.exe --update` ならいつでも確認できます。
- 更新の保存と適用は `launcher/src/staging.rs` の手順で行います。インストーラーはメモリーに溜めずに `Update.tmp-<pid>-<n>` へ直接書き込み（400MB まで）、ファイルから署名を確かめてから、署名・版と一緒に名前の変更で `Update` と入れ替えます。`Update` は「無い・前の一式・新しい一式」のどれかで、途中で止まっても半端な状態にはなりません。保存と適用は `StillBrowser\Update.lock` のロックで排他します。起動時はロックを待たず、使用中なら更新を後回しにして普通に起動します。確認の処理は待ってから、保存済みやインストール済みの版より新しいときだけ保存します。適用時もファイルから署名を確かめ、確認中と起動まではインストーラーを書き換えできないように開いたままにします。
- Mozilla 配布の Firefox は、Firefox の自動更新で新しくなります。Mozilla 版のインストーラーは、既存の Mozilla 版より同梱の Firefox の方が新しいときに置き換えます。既存が独自ビルドの場合は、同梱の版が同じか古い場合も置き換えて自動更新を戻します。
- Still 独自のビルド（`--gecko`）は、自分では更新しません（更新プログラムを外し、ポリシー `DisableAppUpdate` も設定します）。インストーラーは `updater.exe` の有無で Mozilla 版と区別し、実行ファイルが存在する独自ビルドで `application.ini` の `BuildID` も同じときだけ残します。同じ BuildID の Mozilla 版も置き換えます。`--gecko` に `updater.exe` を含む zip を渡すと、パッケージ作成を中止します。ESR のセキュリティ更新（およそ4週ごと）のたびに、ビルドし直して配布してください。既存の Firefox を残す場合も、Still の AutoConfig・設定画面・ウィンドウアイコンは必ず更新します。これらは Mozilla のファイルとは別に追加しているため、Firefox の自動更新でも残ります。

## ビルド（Linux から）

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/still-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
python3 windows/package.py            # x64 と arm64、Mozilla の最新 ESR
python3 windows/package.py --arch x64 --firefox 153.5.0esr
```

- 必要なもの：`cargo-xwin`（`~/.local/llvm18/bin` の clang・lld-link・llvm-rc）、`makensis`（`~/.local/nsis`）、署名用の Tauri CLI（`desktop/ui/node_modules`、`cd desktop/ui && npm ci`）。
- Firefox は Mozilla の更新用パッケージ（`complete.mar`）から取り出します。`SHA256SUMS` と照合し、`windows/cache/` に置きます。
- 版は `launcher/Cargo.toml` の `version` です。Linux 版と Android 版もこの版を使います。上げてから3つともビルドし、`python3 site/build.py` を実行します。

### メモリ使用量を減らした Firefox（x64）

Mozilla の ESR ソースに `gecko/patches/` を当て、Linux から Windows 向けにクロスビルドします。

```sh
python3 windows/gecko/build.py --firefox 153.5.0esr          # 最後に日本語版の zip のパスを出力
python3 windows/package.py --arch x64 --firefox 153.5.0esr --gecko <出力された zip>
```

- ソースは `SHA256SUMS` と照合して `~/still-gecko/`（`STILL_GECKO_HOME` で変更可）に展開します。ツールチェーン（clang・Windows SDK・Rust）は mach が `~/still-gecko/mozbuild` に取得します。日本語は、Mozilla がその版に使った `firefox-l10n` のリビジョンから作り直します。
- 先に `sudo apt install msitools unzip zip m4 libc6-i386` が必要です（Windows SDK の展開、configure、パッケージ作成、MIDL を動かす wine の 32 ビット部分に使います）。
- 空きディスクが約 50GB と、RAM 7GB ならスワップ 16GB が必要です。並列数は 6 で、LTO と PGO は使いません。初回は数時間かかります。
- 変更点：
  - `0001-memory-defaults.patch`：
    - 共有コンテンツプロセスを 8 から 4 に減らします（サイト分離の Fission はそのまま）。
    - 待機用のプロセスと、隠れて読み込んでおく新しいタブを作りません。
    - 戻る・進むで保持するページを 2 にします。
    - メモリーキャッシュを 32MB、画像キャッシュを 512MB までにします。
    - 空きメモリーが 1GB を切ったら背景のタブを解放します。
    - ML エンジン・AI チャット・Normandy を止めます。
  - `0002-jemalloc-dirty-pages.patch`：解放済みでもまだ OS に返していないページの上限を、アリーナあたり 256 から 64 ページに下げます。
  - ブランディング：`build.py` が Mozilla の非公式ブランディングを複製し、名前とアイコンを Still にします。改変したビルドには Firefox の名前とロゴを使えないためです。
- `--safebrowsing-key` に Google Safe Browsing の API キーのファイルを渡さないと、フィッシング・マルウェアのリストが更新されません（Mozilla の公式ビルドだけが Mozilla のキーを持っています）。
- 新しい ESR でパッチが当たらないときは、`build.py` がどのパッチかを示して止まります。

## テスト

```sh
cd windows/launcher
cargo test --offline                                         # プロファイル・移行・更新の判定・署名
STILL_INSTALLER=../../artifacts/Still_0.5.1_x64-setup.exe cargo test -- --ignored   # 作ったインストーラーの署名
cargo xwin clippy --release --target x86_64-pc-windows-msvc -- -D warnings
cargo xwin clippy --release --target aarch64-pc-windows-msvc -- -D warnings
```

`staging.rs` の更新の手順（中断・サイズ超過・署名違い・古い版・試行は一度・ロック）は Linux の `cargo test` で確かめます。

### E2E 用のビルド（`--features e2e`）

既定では無効の Cargo の機能 `e2e` を付けたビルドだけが、次の環境変数を読みます。リリース用のビルド（`package.py`）には入りません。

| 環境変数 | 内容 |
|---|---|
| `STILL_UPDATE_MANIFEST_URL` | `update/windows.json` の代わりに読むマニフェスト |
| `STILL_UPDATE_PUBLIC_KEY` | 署名を確かめる公開鍵（minisign の公開鍵ファイルの base64。`tauri signer generate` の `.pub` と同じ形式） |
| `STILL_E2E_LOG` | ダイアログを出さずに、このファイルへ1行ずつ追記します（`pid<TAB>info/warning/question<TAB>答え<TAB>本文`、改行は `\n`） |
| `STILL_E2E_ANSWER` | `STILL_E2E_LOG` があるときの「はい／いいえ」の答え（`yes`（既定）か `no`） |

`e2e` のビルドは `http://127.0.0.1` と `http://localhost` からのダウンロードも受け付けます。インストール済みの版は `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Still` の `DisplayVersion` が `still.exe` より新しければそちらを使います。

リポジトリのルートから `python3 -m unittest discover -s windows/tests -v` で、ネットワークを使わず配布用ファイルの追加と Mozilla のファイルが変わらないことを確認します。`node --test windows/tests/*.test.cjs` では、Firefox の API を置き換えて、各ウィンドウのタイトルと設定画面の操作を確認します。

`xvfb-run -a env STILL_TEST_WINE=/path/to/wine python3 -m unittest discover -s windows/tests -p test_package.py -k InstallerChecks -v` では、NSIS の実インストーラーを Mozilla 版と独自版で作り、一時 Wine 環境へ実行します。同版・旧版への移行、Mozilla の新しい版の保持、BuildID が同じときの判別、Still の追加ファイルと更新機能の復元を13通りの既存ランタイムで確認します。`makensis` と Wine が必要です。Firefox 本体とランチャーには小さいダミーファイルを使い、署名鍵や普段のインストール先には触れません。

`xvfb-run -a .venv-linux/bin/python windows/tests/newtab_browser.py` では、Linux の Firefox を一時フォルダーに複製し、Windows 用の AutoConfig と新規タブ画面を使って検証します。Firefox の従来のホーム設定画面で空白ページを選ぶ操作と Ctrl+T、設定の往復、拡張機能による新規タブの上書きと削除を確認します。`linux/tests/requirements.txt` の Selenium、`.tools/geckodriver`、`linux/runtime/firefox/firefox` が必要です（実行ファイルは引数でも指定可能）。ログ・画像・結果は `artifacts/windows-newtab-checks/` に保存します。Windows 実機での検証は別途必要です。

`cargo run -- --prepare-profile DIR` で Windows 用のプロファイルを Linux 上に作れます。以前のツールバーの並びと取り込みに加え、設定画面（設定ボタン・≡ メニューの「設定」から開く、検索エンジンの一覧、テーマの切り替え、版の表示）とウィンドウのタイトルは、Linux 版の Firefox 153.5.0esr に同じファイルを入れて確認しています。ウィンドウのアイコン（`.ico`）、閲覧データの削除・取り込み・パスワードの各画面、`still.exe` を呼ぶ画像の選択と更新、Firefox 自動更新後の追加ファイルの保持は、Windows 実機での確認が必要です。既定のブラウザにする項目はありません。Firefox の仕組みでは `firefox.exe` が登録され、外部リンクが Still 専用のプロファイルで開く保証がないためです。
