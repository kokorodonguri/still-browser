# Still for Linux

Mozilla Firefox ESR本体に、縦タブ、ニュートラルグレーとシステムブルーのUI、ホーム画面のスタイルを加えたLinux x86_64版です。Web表示エンジンはGeckoです。Firefox本体のソースコードを改変したフォークではありません。CSSと設定はこのフォルダーで管理しています。

## 起動

配布用tar.gzを展開した場合は、その中の `./still` を起動します。Firefoxは同梱されています。

リポジトリからの場合:

```sh
python3 linux/install.py
./linux/still
```

Ubuntu 24.04で必要なシステムライブラリの例:

```sh
sudo apt install libgtk-3-0t64 libnss3 libasound2t64 libdbus-glib-1-2
```

通常のLinuxデスクトップ上で起動できます。画面のない環境で操作テストを行う場合はXvfbを使います。

アプリメニューへの登録は `python3 linux/install-desktop.py` で行います。

## 使い方

- アドレス欄にURLまたは検索語を入力します。検索エンジンはGoogleです。入力中は履歴・ブックマーク・開いているタブと、Googleの検索候補が出ます（0.4.0までのプロファイルも、0.5.0の初回起動時に検索候補を有効にします。止めるには Firefox の設定 › 検索 › 「検索候補を表示する」を外します）。
- 左の縦タブまたはCtrl+Tabで切り替え、Ctrl+Tでタブを追加します。
- Ctrl+Dでブックマークを追加、Ctrl+Shift+Oで一覧を表示します。
- Ctrl+Hで履歴、Ctrl+Shift+Yでダウンロード一覧を表示します。
- ダウンロードは通常のDownloadsフォルダーに保存されます。保存先と毎回確認する設定はFirefoxの設定画面から変更できます。
- 終了後に再起動すると前回のタブを復元します。未選択タブは選択するまで読み込みません。
- 新しいタブ（ホーム）の背景に好きな画像を表示できます。アプリメニューに登録した場合は右クリックの「新しいタブの画像を選ぶ…」、端末からは `./still --newtab-image 画像のパス` で設定し、「新しいタブの画像を外す」か `./still --remove-newtab-image` で元に戻します。`./still --newtab-image` とパスを省くと、ファイル選択画面（GTK、なければzenity／kdialog）が開きます。PNG・JPEG・GIF・WebP・AVIF・BMPの20MBまでです。画像はプロファイルの `chrome/newtab/` にコピーするので、元のファイルを消しても残ります。Firefoxは`userContent.css`を起動時にしか読まないため、Stillの起動中に変えた場合は再起動後に反映されます。

## 他のブラウザから引き継ぐ

アプリメニューに登録した場合は右クリックの「他のブラウザから引き継ぐ…」、端末からは `./still --import` で、Firefox の「ブラウザーデータのインポート」画面を開きます。取り込みが終わるとそのタブは閉じます。Still を起動中のときは、アドレス欄に `about:preferences#general-migrate` と入力すると同じ画面が開きます。

| 取り込み元 | 取り込めるもの |
|---|---|
| Chrome・Chromium・Brave・Opera・Vivaldi | ブックマーク、履歴。Linux の Firefox は Chrome 系のパスワードを読めません |
| 通常の Firefox（他のプロファイル） | ブックマーク、履歴、パスワード、Cookie（ログイン状態）など |
| パスワードの CSV ファイル | Chrome などの設定 › パスワード から書き出したもの。取り込んだあと、書き出したファイルは削除してください |
| ブックマークの HTML ファイル | どのブラウザから書き出したものでも |

Chrome 系の Cookie は取り込めないため、Google などには Still でログインし直してください。

## データと更新

専用プロファイルは `$XDG_CONFIG_HOME/still-browser`、未設定なら `~/.config/still-browser` です。通常のFirefoxのプロファイルとは別に保存します。`./still --profile /任意のディレクトリ` で保存場所を変更できます。

サイドバー、ダウンロード先、起動方法などの初期設定は初回だけ適用します。その後のユーザーの選択はFirefoxに保存されます。テーマの読み込みとホーム検索の表示方式は、Stillのインターフェースに必要な設定として維持します。

ホーム検索の準備中は案内が表示され、入力欄は操作できません。検索エンジン情報が揃うと入力できるようになります。アドレス欄からの検索はこの待ち時間にも利用できます。

Still 自体の更新：配布版（`release.json` を含むもの）は、起動するたびに `https://still.dongurihub.com/update.json` を確認します。新しいバージョンがあるとデスクトップ通知が出て、「更新」を押すとダウンロードし、SHA-256 を確かめてから `<展開したフォルダー>.update` に展開します。入れ替えは次に起動したとき、Firefox が動いていない状態で行います。端末からは `./still --update`、アプリメニューに登録した場合は右クリックの「アップデートを確認」でも確認できます。ソースから起動した場合（`release.json` なし）は更新しません。配布用アーカイブのバージョンは Windows 版と同じで、`package.py` が `windows/launcher/Cargo.toml` の `version` を読みます。

Firefoxは公式の更新機能を維持しています。Still の更新でも、自動更新された Firefox は、配布物に含まれる Firefox の方が新しい場合を除いてそのまま使います。`python3 install.py` でMozillaの最新ESRを取得し直すこともできます。MozillaのSHA-256と照合してから展開します。更新時もプロファイルは保持します。FirefoxのUI更新に伴いCSSの調整が必要になる場合があります。

通常起動では自動操作のポートを開きません。テストだけでSeleniumとgeckodriverを使用します。

## 操作確認

```sh
python3 -m venv .venv-linux
.venv-linux/bin/pip install -r linux/tests/requirements.txt
# Mozilla公式geckodriverをPATHに配置して実行します。
xvfb-run -a -s '-screen 0 1600x1000x24' .venv-linux/bin/python linux/tests/verify.py
.venv-linux/bin/python linux/tests/test_profile.py
python3 linux/tests/test_updater.py
python3 linux/tests/test_newtab_image.py
xvfb-run -a -s '-screen 0 1600x1000x24' .venv-linux/bin/python linux/tests/newtab_image.py
xvfb-run -a -s '-screen 0 2560x1600x24' .venv-linux/bin/python linux/tests/review_ui.py
```

テストは一時プロファイルとローカルHTTPサーバーで実行し、実際のFirefox UIのアドレス欄・ナビゲーション・縦タブ・ブックマーク・履歴・ダウンロード・再起動後の遅延復元を確認します。結果と画面は `artifacts/linux-checks/` に保存します。

通常起動の画面撮影には `linux/tests/capture.py` を使います。テスト用の一時プロファイルを起動し、`capture 名前.png` でX11の画面をそのまま保存します。Seleniumの操作中を示す帯がない通常のFirefox画面も確認できます。

この環境はユーザー名前空間の作成を許可せず、Firefoxがsandboxの警告を表示します。sandboxを無効にする設定は追加していません。操作テストでは警告を記録し、スクリーンショット撮影のために通知を閉じています。Waylandと他のLinuxディストリビューションでの動作確認は未実施です。

Firefoxのウィンドウ最小幅はこの環境で約732pxです。320px・390pxを指定したテストでも実際の幅は732pxでした。デスクトップ向けであり、スマートフォンでの実動作は対象にしていません。

配布用アーカイブは `python3 linux/package.py` で生成します。Firefoxは [Mozillaの公式配布](https://archive.mozilla.org/pub/firefox/releases/) を使い、検索エンジンの指定は [Firefoxの公式ポリシー](https://mozilla.github.io/policy-templates/#searchengines--default) を使っています。


## Appearance

Apple HIGの階層、視認性、マテリアルの原則を参考にしたデスクトップUIです。単色の背景、微細な境界線、24pxの背景ぼかし、弱い内側・外側の影を使います。青はフォーカスや操作のアクセントです。ホーム画面やサイドバーに装飾用グラデーションはありません。

Firefoxの配色設定に応じてライト／ダークが切り替わります。既存プロファイルの配色設定を保持します。フォントはAppleシステムフォントを優先し、LinuxではNoto Sansなど利用可能なフォントにフォールバックします。SFフォントやSF Symbolsは同梱していません。キーボードフォーカス、コントラスト強調、動きと透明効果の低減設定に配慮しています。

これはLinux上のFirefox UIです。Apple純正のAppKit/UIKitコントロールやLiquid Glass、OSによるウィンドウ背後のぼかし、iOS/iPadOS対応を実装したものではなく、HIGへの完全準拠を保証するものでもありません。ぼかしはFirefox内で背後に描画された内容に適用されます。Windows版は同じダーク配色で、透明マテリアルには対応していません。

外観検証: `xvfb-run -a .venv-linux/bin/python linux/tests/appearance.py`（リポジトリ直下）。ライト／ダーク、検索欄のフォーカス、ホームの横幅、背景色とグラデーションの不使用を実際のFirefoxで検証し、`artifacts/linux-checks/apple-*.png`に画面を保存します。
