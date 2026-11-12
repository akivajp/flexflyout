# Microsoft Store の申請内容

Partner Center（日本語の画面）で FlexFlyout を申請するときに入れる内容です（アプリとゲーム → FlexFlyout → 申請の開始）。Markdown を表示した状態からコピーして、欄に貼っていく想定で書いています。1 つずつ欄に入れるもの（製品の機能、検索用語、キャプション）はリストにしています。掲載情報の文は README と内容をそろえてください。

## 価格と使用可能状況

- **市場**: すべての市場
- **表示**: 一般公開。Store で見つけられるようにする
- **スケジュール**: できるだけ早く（実際の公開は、下の「申請オプション」の公開の保留で決める）
- **価格**: 無料。試用版なし、セールなし

## プロパティ

- **カテゴリ**: ユーティリティ & ツール
- **プライバシー ポリシー**: 「個人情報を使用する」を選び、次の URL を入れる。FlexFlyout は何も保存しないが、キーボードの低レベルのフックは仕組み上すべてのキーの通知を受け取る（見るのは音量キーとメディアキーだけ）。Store の方針では「個人情報へのアクセス」と見なされうるため。README のプライバシーの節に、していることを正確に書いてある
  - https://github.com/akivajp/flexflyout#privacy
- **Web サイト**: https://github.com/akivajp/flexflyout
- **サポートの連絡先情報**: https://github.com/akivajp/flexflyout/issues
- **製品の公表**: デフォルトのまま
  - オフ: Store の商取引を使わない購入（購入の機能は無い）、アクセシビリティのテスト済み（Store のチェックリストで確かめていない）、ペンとインク、生成 AI の機能（アプリの機能として無い。開発に AI を使ったことは関係ない）
  - オン: 代替ドライブへのインストール、OneDrive へのバックアップ（設定ファイルと小さなログだけで、PC を移したときに設定が戻る）、録画とブロードキャスト（ゲーム向けで影響なし）
- **システム要件**: Windows 11 のほかは無し（音量キーやメディアキーのあるキーボードがあるとよいが、音量が変わればいつでも表示する）

## 年齢別レーティング

- **アンケート**: 「International Age Rating Coalition (IARC) のアンケートへ回答する準備ができています」
- **アプリの種類**: 「その他のすべてのアプリの種類」（例にユーティリティ・ツール・カスタマイズが入っている。「ソーシャルまたはコミュニケーション」ではない）
- **評価機関から直接取得した評価を使用するか、物理メディアで配布するか**: いいえ
- **その後の質問**: すべて「いいえ」（暴力・性的な表現・言葉づかい・ギャンブル・薬物なし、利用者どうしのやり取りや共有なし、位置情報の共有なし、デジタルの購入なし、自由な Web の閲覧なし）。結果はどの地域でも一番低い区分（IARC 3+）になるはず

## パッケージ

- リリースのワークフローの成果物 `flexflyout-msix` の `flexflyout-vX.Y.Z-x64.msix` と `-arm64.msix` を上げる（`-test` の付いたものではない。最初の申請の後は、リリースのワークフローが API で上げる）
- デバイス ファミリ: Windows 10/11 デスクトップ（マニフェストでは Windows 11 以上を指定）

## 申請オプション

- **公開の保留オプション**: 初回は「[今すぐ公開] を選択するまで、この提出物を公開しない」。告知の準備ができてから公開する
- **認定の注意書き**: この画面ではなく、リンク先の「追加のテスト情報」のページに、次の英語の文を貼る

  > FlexFlyout is a tray (notification area) app without a main window. To see it, change the volume (keyboard keys or the volume slider), press a media key, or start playing something in Media Player or a web browser: the flyout appears at the top right of the primary monitor for a few seconds. Its tray icon (in the overflow area behind the ^ next to the clock) has a menu with "Show the flyout", "Settings…" and "Quit". No account, sign-in or test data is needed.

- **制限付き機能**（`runFullTrust` の理由。**500 文字まで**）: 次の英語の文を貼る

  > FlexFlyout is a desktop (Win32) app. It needs full trust for a low-level keyboard hook that watches only the volume and media keys (to show the flyout even at 0% or 100% volume), Core Audio (volume), the media sessions (SMTC), a tray icon, and a topmost window that takes no focus. It also finds Windows' volume indicator to keep clear of it, and hides it only if the user asks to (restored when FlexFlyout quits).

## ストア登録情報

英語（米国）と日本語の 2 つを追加し、それぞれにその言語のスクリーンショットを入れる（PNG、1366×768 以上、3840×2160 まで）。

スクリーンショットは `packaging/msix/store/screenshots/` にある。共有の Windows 11 の仮想マシンで、試験用の MSIX（Store 版と同じく、設定ウィンドウに新しい版の確認の項目が無い）を入れて撮った。曲は架空の曲と自作のジャケットを Edge で流し（プレーヤーの名前がどの言語でも「Microsoft Edge」になる）、壁紙も自作。評価版の透かしとタスクバーを外して 16:9 に切り抜き、1366×768 に拡大した。番号の順に登録する。

| 順 | 英語 | 日本語 |
| --- | --- | --- |
| 1 | `en-1-flyout.png` | `ja-1-flyout.png` |
| 2 | `en-2-settings.png` | `ja-2-settings.png` |
| 3 | `en-3-new-track.png` | `ja-3-new-track.png` |
| 4 | `en-4-light.png` | `ja-4-light.png` |

キャプション（「イメージの字幕の追加」、200 文字まで）は、各言語の TOML ファイルにある。

Microsoft Store ロゴ（任意）:

- **1:1 のボックス アート**: 登録をおすすめ（Store のいろいろな一覧で使われる）。`tmp/store-shots/store-box-2160.png`（`tmp/make_store_art.py` でアイコンの SVG から作成）
- **9:16 のポスター アート**: 必要なのは Xbox だけ。入れるなら `tmp/store-shots/store-poster-1440x2160.png`
- **タイル アイコン（300 / 150 / 71）**: 空のまま（パッケージのアイコンが使われる）

### 掲載情報の文とスクリーンショット

英語（米国）・日本語の掲載情報（製品名・短い説明・説明・このバージョンの新機能・製品の機能・検索用語・著作権・追加のライセンス条項・開発元・スクリーンショットとキャプション）は、`packaging/msix/store/en-us.toml`・`ja-jp.toml` が正本。

- Store へは Microsoft Store のワークフローで反映するので、画面に貼らなくてよい。タグを打つとリリースのワークフローが、その版の MSIX と掲載情報で**下書き**の申請を作る。変わる項目と新しい文はそのジョブのログに出るので、それを見て、Microsoft Store のワークフロー（手動の実行）の `commit` で提出する。
- **API で作った申請は、提出するまで Partner Center の画面に変更が出ない**。画面で変えたり「送信して認定を受ける」を押したりすると、その申請は API で扱えなくなり、変更前の内容で出る恐れがある。画面では触らない（2026-11-10 に確認）。
- **リリースのたびに `release_notes`（このバージョンの新機能）を、英語・日本語とも書き換えてからタグを打つ**。
- スクリーンショットを差し替えたときは、Microsoft Store のワークフローを `submit`・`images` を付けて動かす（タグのリリースでは差し替えない）。
- 言語を足すときは、`<言語>.toml`（例: `de-de.toml`）を足す。その言語のスクリーンショットが無ければ `[[screenshots]]` を書かない。
