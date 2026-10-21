# Still Browser

開発者向けの構成・データフロー・問題点とリファクタリング方針は [ARCHITECTURE.md](ARCHITECTURE.md) にまとめています。

Architecture, data flow, review findings and the refactor plan: [ARCHITECTURE.md](ARCHITECTURE.md).

Windows 版は Apple のスタイルリファレンスに沿った白と青のフレーム、Still 独自の縦タブと新しいタブ画面、Linux 版は Apple HIG を参考にしたニュートラルな外観、Android 版は Firefox for Android を参考にした下部ツールバーとプレビュー付きのタブ一覧を備えたブラウザです。どの版も Firefox のエンジン（Gecko）でページを表示します。Windows 版と Linux 版は Firefox ESR を同梱し、Android 版は GeckoView を使います。

アドレス欄に入力すると、履歴・ブックマーク・開いているタブと、検索エンジンの候補が出ます（Windows・Linux は Firefox のアドレスバー、Android は Still の候補一覧）。

## ダウンロードページ

`python3 site/build.py --serve` で `artifacts/` の配布ファイルを並べたページを生成し、ポート 8424 で配信します。公開アドレスは https://still.dongurihub.com/ （Cloudflare Tunnel で `http://127.0.0.1:8424` に転送）です。待ち受けアドレスは `--host` で変更できます。生成物は `site/public/` に出力します。

## 他のブラウザから引き継ぐ

| アプリ | 開き方 | ブックマーク・履歴 | パスワード | Cookie（ログイン状態） |
|---|---|---|---|---|
| Windows | Still の設定 › 引き継ぎ／`still.exe --import`／`about:preferences#general-migrate` | Chrome・Edge・他の Firefox などから（Firefox の取り込み機能）。以前の Still（Tauri 版）のブックマークと履歴は初回起動時に自動で引き継ぎます | Firefox の取り込み機能、または CSV | 他の Firefox からのみ |
| Android | 設定 › 他のブラウザから引き継ぐ | 書き出したブックマーク（HTML）、Google Takeout の履歴（JSON） | Android の自動入力（Google パスワードマネージャーなど）が入力します | 不可 |
| Linux | アプリメニューの右クリック「他のブラウザから引き継ぐ…」／`./still --import` | Chrome・Chromium・Brave・Opera・Vivaldi・他の Firefox プロファイルから（Firefox の取り込み機能） | 他の Firefox プロファイルから、または CSV | 他の Firefox プロファイルからのみ |

Chrome と Edge は、Cookie とパスワードをそのブラウザ専用に暗号化しています（App-Bound 暗号化）。外部から取り出すのはマルウェアと同じ手口なので、Still は読みません。パスワードは Chrome / Edge の設定 › パスワード から CSV に書き出して取り込み、書き出したファイルは取り込んだあと削除してください。Google などには Still でログインし直します。

## アップデート

各アプリは起動時にこのダウンロードページへ新しいバージョンを確認し、あれば通知します。ユーザーが「更新」を押すまでダウンロードしません。設定画面からも確認できます。

| アプリ | 確認する場所 | 通知と手動確認 | 適用のしかた |
|---|---|---|---|
| Windows | `update/windows.json` | 起動時のダイアログ／Still の設定 › 情報／`still.exe --update` | 署名を検証したインストーラーを、次に起動したとき（Firefox が動いていないとき）に実行します。以前の Tauri 版も同じ manifest から更新します |
| Android | `update.json` | 画面上部の通知の「更新」／設定 › 情報 › アップデートを確認 | SHA-256 を検証した APK をシステムのインストーラーに渡します。初回だけ「この提供元のアプリを許可」を求められます |
| Linux | `update.json` | デスクトップ通知の「更新」／アプリメニューの右クリック「アップデートを確認」／`./still --update` | SHA-256 を検証して展開し、次に起動したときに入れ替えます。自動更新済みの Firefox は、配布物の方が新しくない限りそのまま使います |

ダウンロードページは HTTP です。Windows はインストーラーの署名（鍵は下記）、Android は APK の署名（インストール済みのアプリと同じ鍵でないと更新できません）で改ざんを防ぎます。Linux は `update.json` の SHA-256 との照合だけなので、同じネットワーク内での改ざんは防げません。

ソースから起動した Linux 版（`release.json` がないもの）は更新しません。

### 新しいバージョンを配布する手順

1. `windows/launcher/Cargo.toml` の `version` を上げます（下げたり同じにしたりすると、更新として扱われません）。Windows・Linux・Android はすべてこの版になります
   - Linux：`linux/package.py` がこの版を `release.json` に書きます
   - Android：`android/app/build.gradle.kts` がこの版を `versionName` にし、`versionCode` は「メジャー×10000＋マイナー×100＋パッチ」（0.5.1 なら 501）にします。マイナーとパッチは 99 までです
2. 3つともビルドし、`artifacts/` に置きます（1つだけ古いと、`site/build.py` が知らせます）。Windows は署名用の環境変数を付けて `python3 windows/package.py` を実行します（`.exe.sig` と、署名とインストーラーの SHA-256 を結び付ける `.exe.release.json` も `artifacts/` に出ます。これがない以前の `.exe` と `.exe.sig` は、`python3 windows/package.py --seal` で記録します）

   ```sh
   export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/still-updater.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
   ```

3. `python3 site/build.py` を実行します。`update.json` と `update/windows.json` も作り直されます。配布ファイルは書き換えられない `dl/<版>/<SHA-256の先頭16桁>/<ファイル名>` に置かれ、マニフェストとページはそこを指します（`dl/<ファイル名>` は人向けの最新版リンク）。ひとつ前の世代のファイルは、ダウンロード中の人のために残します。失敗したときは、以前のマニフェストとファイルがそのまま残ります

Windows の更新署名の秘密鍵は `~/.tauri/still-updater.key`（パスワードなし、リポジトリ外）です。なくすと、配布済みの Windows 版は以後更新できなくなります。バックアップしてください。GitHub Actions で署名する場合は、リポジトリの Secrets に `TAURI_SIGNING_PRIVATE_KEY` を登録します。

配信先のアドレス（`https://still.dongurihub.com/`）は、`site/build.py` の `PUBLIC_URL`、`windows/launcher/src/update.rs` の `MANIFEST_URL`、`android/app/build.gradle.kts` の `update_url`、`linux/updater.py` の `UPDATE_URL` にあります（以前の Tauri 版は `desktop/src-tauri/tauri.conf.json` の `plugins.updater.endpoints` を見ます）。変えるときはすべて揃えます。

## Android

`artifacts/StillBrowser-Android.apk`（約90MB、Android 10 以上、64ビット ARM 端末）をインストールします。ページは GeckoView（Firefox のエンジン）で表示します。使い方・ビルド・テストは [android/README.md](android/README.md) を参照してください。

下部のアドレス欄で検索・URL 入力を行い、メニューからブックマーク・履歴・ダウンロード・設定を開きます。タブの数を押すかアドレス欄を上にスワイプするとプレビュー付きのタブ一覧が開きます。新しいタブには Still のロゴと、ブックマーク・よく訪れるホストのショートカットを最大8件表示します。背景画像も引き続き選べます。

## Linux

配布用 `artifacts/StillBrowser-linux-x64.tar.gz` を展開して `./still` を起動します。Firefoxを同梱しています。

ソースから起動する場合:

```sh
python3 linux/install.py
./linux/still
```

設定・配布・操作確認の説明は [linux/README.md](linux/README.md) を参照してください。

## Windows

`artifacts/Still_<版>_x64-setup.exe`（ARM64 版は `_arm64-setup.exe`）を実行します。Firefox ESR を同梱した Still をユーザーごとにインストールします（管理者権限は不要）。コード署名はしていないため、SmartScreen の警告が出ることがあります。使い方・移行・ビルドは [windows/README.md](windows/README.md) を参照してください。

ウィンドウのアイコンとタイトルは Still のものです。タブ欄の一番下の設定ボタンと … メニューから、Still 独自の設定画面を開けます。起動時のタブ・テーマ・検索・履歴の設定、新しいタブの画像、引き継ぎ、更新確認をまとめています。新規プロファイルはライトテーマで始まり、Windows の配色に従うかダークを選ぶこともできます。画面は、Firefox に設定画面やアイコンを追加する作りです。Firefox は、メモリ使用量を減らすパッチを当てた Still 独自のビルド（[windows/README.md](windows/README.md)）も使えます。Firefox の利用規約の確認も残しています。

### 以前の Windows 版（Rust + Tauri / WebView2）

0.4.0 までの Windows 版です。ソースは `desktop/`、使い方・ビルド・テストは [desktop/README.md](desktop/README.md)、設計は [desktop/DESIGN.md](desktop/DESIGN.md) にあります。もう配布していませんが、インストール済みの 0.4.0 は自動更新で Firefox 版に移ります。ブックマーク・履歴・新しいタブの画像は引き継ぎ、保存したパスワードとログイン状態は引き継ぎません。

### 旧版（C# / WPF）

ルート直下の `MainWindow.xaml` などが旧版のソースです。比較用に `artifacts/StillBrowser-Windows-*-wpf.exe` として残しています。


単一EXE版は `artifacts/StillBrowser-Windows-x64.exe` を実行します。.NETランタイムとアプリのDLLを同梱しており、初回起動時に必要なネイティブライブラリを展開します。Microsoft Edge WebView2 Runtimeは別途必要です。Windows版のエンジンはWebView2です。

フォルダー版はWindows x64で配布用フォルダーの `LightweightBrowser.exe` を実行します。フォルダー内のファイルはまとめて配置してください。.NETランタイムは配布物に含まれていますが、Microsoft Edge WebView2 Runtimeは必要です。

#### 旧版のビルド

.NET 8 SDKを使います。

```sh
dotnet build -c Release -p:EnableWindowsTargeting=true
dotnet publish -c Release -r win-x64 --self-contained true -p:EnableWindowsTargeting=true -o artifacts/win-x64
```

この作業環境ではSDKを `/home/donguri0423/.dotnet` に導入しています。PATHが更新される前のシェルでは `/home/donguri0423/.dotnet/dotnet` を使えます。

ブックマーク、履歴、タブ情報は `%LOCALAPPDATA%\StillBrowser\state.json` に保存します。

Linux版はUbuntu 24.04 / Xvfb上の実Firefoxで操作確認しています。Windows版はコンパイルと配布用ファイルの生成を確認済みです。Windows上での起動・操作確認は未実施です。

画面は極小構成です。サイドバーは初期状態で閉じ、新規タブは空白で、検索・URL入力は上部のアドレス欄に集約しています。縦タブはサイドバー切替ボタンで開きます。Windows版は `Ctrl+L` / `Ctrl+T` / `Ctrl+W` / `Ctrl+Tab` / `Ctrl+Shift+Tab` に対応し、サイドバーの開閉状態を保存します。既存のLinuxプロファイルは初回起動時だけサイドバーを閉じた状態に移行し、元のセッションを `sessionstore.jsonlz4.before-minimal` にバックアップします。

外観変更の確認結果は [UIレビュー](artifacts/linux-checks/APPLE-UI-REVIEW.md)、通常起動の画面は [スクリーンショット](artifacts/linux-checks/apple-native-dark.png) を参照してください。

単一EXE版の生成: `dotnet publish -c Release -r win-x64 --self-contained true -p:EnableWindowsTargeting=true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:EnableCompressionInSingleFile=true -o artifacts/windows-singlefile`。生成された `LightweightBrowser.exe` を配布名 `StillBrowser-Windows-x64.exe` としてコピーします。

Windows ARM64向けの単一EXEは `artifacts/StillBrowser-Windows-arm64.exe` です。同じpublishコマンドのRIDを `win-arm64`、出力先を `artifacts/windows-arm64-singlefile` に変更して生成します。.NETを同梱し、WebView2 Runtimeは別途必要です。ARM64実機での動作確認は未実施です。
