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

- リリースのワークフローの成果物 `flexflyout-msix` の `flexflyout-vX.Y.Z-x64.msix` を上げる（`-test` の付いたものではない）
- デバイス ファミリ: Windows 10/11 デスクトップ（マニフェストでは Windows 11 以上を指定）

## 申請オプション

- **公開の保留オプション**: 初回は「[今すぐ公開] を選択するまで、この提出物を公開しない」。告知の準備ができてから公開する
- **認定の注意書き**: この画面ではなく、リンク先の「追加のテスト情報」のページに、次の英語の文を貼る

  > FlexFlyout is a tray (notification area) app without a main window. To see it, change the volume (keyboard keys or the volume slider), press a media key, or start playing something in Media Player or a web browser: the flyout appears at the top right of the primary monitor for a few seconds. Its tray icon (in the overflow area behind the ^ next to the clock) has a menu with "Show the flyout", "Settings…" and "Quit". No account, sign-in or test data is needed.

- **制限付き機能**（`runFullTrust` の理由。**500 文字まで**）: 次の英語の文を貼る

  > FlexFlyout is a desktop (Win32) app. It needs full trust for a low-level keyboard hook that watches only the volume and media keys (to show the flyout even at 0% or 100% volume), Core Audio (volume), the media sessions (SMTC), a tray icon, and a topmost window that takes no focus. It also finds Windows' volume indicator to keep clear of it, and hides it only if the user asks to (restored when FlexFlyout quits).

## ストア登録情報

英語（米国）と日本語の 2 つを追加し、それぞれにその言語のスクリーンショットを入れる（PNG、1366×768 以上、3840×2160 まで）。

スクリーンショットは手元の `tmp/store-shots/`（コミットしない）にある。共有の Windows 11 の仮想マシンで、試験用の MSIX（Store 版と同じく、設定ウィンドウに新しい版の確認の項目が無い）を入れて撮った。曲は架空の曲と自作のジャケットを Edge で流し（プレーヤーの名前がどの言語でも「Microsoft Edge」になる）、壁紙も自作。評価版の透かしとタスクバーを外して 16:9 に切り抜き、1366×768 に拡大した。番号の順に登録する。

| 順 | 英語 | 日本語 |
| --- | --- | --- |
| 1 | `en-1-flyout.png` | `ja-1-flyout.png` |
| 2 | `en-2-settings.png` | `ja-2-settings.png` |
| 3 | `en-3-new-track.png` | `ja-3-new-track.png` |
| 4 | `en-4-light.png` | `ja-4-light.png` |

キャプション（「イメージの字幕の追加」、200 文字まで）は、各言語の掲載情報の節にある。

Microsoft Store ロゴ（任意）:

- **1:1 のボックス アート**: 登録をおすすめ（Store のいろいろな一覧で使われる）。`tmp/store-shots/store-box-2160.png`（`tmp/make_store_art.py` でアイコンの SVG から作成）
- **9:16 のポスター アート**: 必要なのは Xbox だけ。入れるなら `tmp/store-shots/store-poster-1440x2160.png`
- **タイル アイコン（300 / 150 / 71）**: 空のまま（パッケージのアイコンが使われる）

### English (United States)

**Product name**: FlexFlyout

**Short description**:

> A media flyout for Windows 11: the playing track, playback controls and the volume when you change the volume, on the monitor, at the place and in the size you choose.

**Description**:

> FlexFlyout shows what's playing when you change the volume or press a media key: the cover art, the title and artist, a seek bar, the playback controls and the volume, in a flyout that fades away after a few seconds. When a new track starts, a small panel shows it; click it for the whole flyout.
>
> Monitors differ from desk to desk, so everything about the flyout can be adjusted:
> • Which monitor: the primary one, the one with the mouse pointer, or a specific one
> • Where: nine anchor points, plus offsets in pixels
> • How big, how long and how transparent
> • Light or dark (or following Windows), the accent color, the corner roundness, and which parts to show
>
> It works with the apps that show up in Windows' media controls, such as Spotify, web browsers and Media Player.
>
> Also:
> • Keep Windows' own volume indicator, with the flyout placed clear of it, or hide it
> • Follow one player only, so that a video in the browser does not take over
> • Stay off the monitor where a full-screen game is running
> • Seek, shuffle, repeat and mute; drag the volume bar, or scroll over it
> • Pin the flyout to keep it on screen, and drag it anywhere
> • English and Japanese
>
> FlexFlyout lives in the notification area, and its settings window shows each change on the flyout as you make it. It collects no data, and installed from the Microsoft Store it makes no network connections.
>
> FlexFlyout is free and open source (MIT or Apache 2.0): https://github.com/akivajp/flexflyout

**Screenshot captions** (in the order of the screenshots):

- What's playing, with controls and the volume, when you change the volume
- Choose the monitor, the place, the size, the display time and the look
- A small panel for a new track; click it for the whole flyout
- Light or dark, following Windows if you like

**What's new in this version**: First release on the Microsoft Store.

**Product features** (one per line):

- Shows the playing track, playback controls and the volume when you change the volume or press a media key
- A small panel for each new track
- Choose the monitor: primary, under the mouse pointer, or a specific one
- Nine anchor points plus pixel offsets
- Adjustable size, display time and opacity
- Light or dark (or following Windows), accent color, corner roundness
- Keep Windows' volume indicator (placed clear of it) or hide it
- Follow one player only
- Stays off full-screen games on that monitor
- Seek bar, shuffle, repeat, mute, volume by dragging or scrolling
- Pin and drag the flyout
- Starts with Windows if you like
- English and Japanese
- Free and open source

**Search terms** (7 at most, one per box):

- volume flyout
- media flyout
- now playing
- volume indicator
- media controls
- music
- OSD

**Copyright and trademark info**: Copyright (c) 2026 Akiva Miura

**Additional license terms**: MIT License or Apache License 2.0, at your option: https://github.com/akivajp/flexflyout#license

**Developed by**: Akiva Miura

### Japanese

**製品名**: FlexFlyout

**短い説明**:

> Windows 11 用のメディアの Flyout。音量を変えると、再生中の曲を操作ボタンと音量と一緒に、選んだモニター・位置・大きさで表示します。

**説明**:

> FlexFlyout は、音量を変えたときやメディアキーを押したときに、再生中の曲のジャケット・曲名・アーティストを、再生位置のバー・再生の操作ボタン・音量と一緒に表示し、数秒で消える Flyout です。新しい曲が始まると小さな表示でお知らせし、クリックすると全体を表示します。
>
> モニターの数や大きさは人それぞれなので、表示のすべてを調整できます。
> • 表示するモニター: メインのモニター、マウスのある画面、番号で指定
> • 位置: 9 か所の基準点と、ピクセル単位のずらし幅
> • 大きさ・表示時間・不透明度
> • 明るい色・暗い色（Windows に合わせることも）、アクセントの色、角の丸み、表示する部品
>
> Spotify、Web ブラウザー、メディア プレーヤーなど、Windows のメディアの操作に表示されるアプリに対応しています。
>
> そのほかにも:
> • Windows 標準の音量表示は、残して重ならない位置に出すか、消すかを選べます
> • 追いかけるプレーヤーを 1 つに決められます（ブラウザーで動画を見ても切り替わりません）
> • 全画面のゲームを動かしているモニターには表示しない設定
> • 再生位置の移動、シャッフル、リピート、ミュート。音量バーはドラッグやマウスのホイールでも
> • ピン留めすると表示したままになり、好きな場所へドラッグできます
> • 英語と日本語に対応
>
> FlexFlyout は通知領域で動き、設定ウィンドウで変えた内容はその場で Flyout に表示されます。データを集めることはなく、Microsoft Store から入れた場合はネットワークにも接続しません。
>
> FlexFlyout は無料のオープンソースです（MIT または Apache 2.0）: https://github.com/akivajp/flexflyout

**スクリーンショットのキャプション**（スクリーンショットの順に）:

- 音量を変えると、再生中の曲を操作ボタンと音量と一緒に表示
- 表示するモニター・位置・大きさ・表示時間・見た目を自由に
- 新しい曲は小さな表示で。クリックで全体を表示
- 明るい色・暗い色。Windows に合わせることも

**このバージョンの新機能**: Microsoft Store での最初の公開です。

**製品の機能**（1 行に 1 つ）:

- 音量を変えたときやメディアキーで、再生中の曲を操作ボタンと音量と一緒に表示
- 新しい曲は小さな表示でお知らせ
- 表示するモニターを選べる（メイン・マウスのある画面・番号）
- 9 か所の基準点とピクセル単位のずらし幅
- 大きさ・表示時間・不透明度を調整
- 明るい色・暗い色（Windows に合わせることも）、アクセントの色、角の丸み
- Windows 標準の音量表示を避けて表示、または消す
- 追いかけるプレーヤーを 1 つに決められる
- 全画面のゲームのモニターには表示しない設定
- 再生位置のバー、シャッフル、リピート、ミュート、音量はドラッグやホイールでも
- ピン留めとドラッグでの移動
- Windows の起動時に開始
- 英語と日本語に対応
- 無料のオープンソース

**検索用語**（7 つまで、1 つの欄に 1 つ）:

- 音量
- フライアウト
- 再生中の曲
- メディア
- 音楽
- OSD
- volume flyout

**著作権と商標の情報**: Copyright (c) 2026 Akiva Miura

**追加のライセンス条項**: MIT License または Apache License 2.0 のいずれかを選択できます: https://github.com/akivajp/flexflyout#license

**開発元**: Akiva Miura
