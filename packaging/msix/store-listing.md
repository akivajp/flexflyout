# Microsoft Store submission

What to enter in Partner Center for FlexFlyout (Apps and games > FlexFlyout > Start your submission). The listing texts are below in English and Japanese; keep them in step with the README.

## Pricing and availability

- **Markets**: all markets.
- **Visibility**: Public audience; make the product discoverable in the Store.
- **Schedule**: as soon as possible (the release itself is held in *Submission options*, below).
- **Pricing**: Free. No free trial, no sale.

## Properties

- **Category**: Utilities & tools.
- **Privacy policy**: choose that the app uses personal information, and give <https://github.com/akivajp/flexflyout#privacy>. FlexFlyout keeps nothing, but its low-level keyboard hook receives every key press (it only looks at the volume and media keys), which the Store's policy may count as accessing personal information; the README's Privacy section says exactly what it does.
- **Website**: <https://github.com/akivajp/flexflyout>
- **Support contact info**: <https://github.com/akivajp/flexflyout/issues>
- **Product declarations**: the defaults. Off: purchases outside the Store's commerce (there are none), "tested to meet accessibility guidelines" (not tested with the Store's checklist), pen and ink, and generative AI features (the app has none; how it was developed does not count). On: installing to alternate drives, OneDrive backup of the app's data (the settings and a small log, which then follow the user to a new PC), and recording clips (meant for games, harmless here).
- **System requirements**: nothing beyond Windows 11 (a keyboard with volume or media keys helps, but the flyout also shows on any volume change).

## Age ratings

The questionnaire: app type **All other app types** (its examples include utilities, tools and customization; not "Social or communication"); "ratings from a rating authority or physical media": No. Answer **No** to everything: no violence, sexual content, language, gambling or drugs; users cannot interact with or share content with each other; no location sharing; no digital purchases; no unrestricted web access. The expected result is the lowest rating everywhere (IARC 3+).

## Packages

Upload `flexflyout-vX.Y.Z-x64.msix` from the release workflow's `flexflyout-msix` artifact (not the `-test` copy). Device family: Windows 10/11 Desktop (the manifest asks for Windows 11).

## Submission options

- **Publishing hold**: "Don't publish this submission until I select Publish now", the first time, to announce it when it is out.
- **Notes for certification**:

  > FlexFlyout is a tray (notification area) app without a main window. To see it, change the volume (keyboard keys or the volume slider), press a media key, or start playing something in Media Player or a web browser: the flyout appears at the top right of the primary monitor for a few seconds. Its tray icon (in the overflow area behind the ^ next to the clock) has a menu with "Show the flyout", "Settings…" and "Quit". No account, sign-in or test data is needed.

- **Restricted capabilities** (`runFullTrust`):

  > FlexFlyout is a desktop (Win32) app. It needs full trust to watch the volume and media keys with a low-level keyboard hook (to show the flyout when they are pressed, even at 0% or 100% volume, where the volume does not change), to read and set the volume through Core Audio, to read the media sessions (SMTC), to show a tray icon, and to show a topmost window that does not take the focus. It also detects Windows' own volume indicator to place the flyout clear of it, and, only if the user chooses so in the settings, hides it while FlexFlyout runs (it comes back when FlexFlyout quits).

## Store listings

Add the listings for **English (United States)** and **Japanese**. Screenshots: PNG, at least 1366×768 (up to 3840×2160), up to 10; the same set fits both languages if the app's language matches (take them with `flexflyout --demo --lang en` and `--lang ja`, which show sample tracks, not real cover art).

Suggested screenshots and captions:

1. The flyout at the top right of a desktop — "What's playing, with controls and the volume, when you change the volume" / 「音量を変えると、再生中の曲を操作ボタンと音量と一緒に表示」
2. The settings window with the flyout as a preview — "Choose the monitor, the place, the size, the display time and the look" / 「表示するモニター・位置・大きさ・表示時間・見た目を自由に」
3. The small panel for a new track — "A small panel for a new track; click it for the whole flyout" / 「新しい曲は小さな表示で。クリックで全体を表示」
4. The light look, or the flyout on a second monitor — "Light or dark, following Windows if you like" / 「明るい色・暗い色。Windows に合わせることも」

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

**What's new in this version**: First release on the Microsoft Store.

**Product features** (one per line):

> Shows the playing track, playback controls and the volume when you change the volume or press a media key
> A small panel for each new track
> Choose the monitor: primary, under the mouse pointer, or a specific one
> Nine anchor points plus pixel offsets
> Adjustable size, display time and opacity
> Light or dark (or following Windows), accent color, corner roundness
> Keep Windows' volume indicator (placed clear of it) or hide it
> Follow one player only
> Stays off full-screen games on that monitor
> Seek bar, shuffle, repeat, mute, volume by dragging or scrolling
> Pin and drag the flyout
> Starts with Windows if you like
> English and Japanese
> Free and open source

**Search terms** (7 at most): volume flyout, media flyout, now playing, volume indicator, media controls, music, OSD

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

**このバージョンの新機能**: Microsoft Store での最初の公開です。

**製品の機能**（1 行に 1 つ）:

> 音量を変えたときやメディアキーで、再生中の曲を操作ボタンと音量と一緒に表示
> 新しい曲は小さな表示でお知らせ
> 表示するモニターを選べる（メイン・マウスのある画面・番号）
> 9 か所の基準点とピクセル単位のずらし幅
> 大きさ・表示時間・不透明度を調整
> 明るい色・暗い色（Windows に合わせることも）、アクセントの色、角の丸み
> Windows 標準の音量表示を避けて表示、または消す
> 追いかけるプレーヤーを 1 つに決められる
> 全画面のゲームのモニターには表示しない設定
> 再生位置のバー、シャッフル、リピート、ミュート、音量はドラッグやホイールでも
> ピン留めとドラッグでの移動
> Windows の起動時に開始
> 英語と日本語に対応
> 無料のオープンソース

**検索用語**（7 つまで）: 音量, フライアウト, 再生中の曲, メディア, 音楽, OSD, volume flyout

**著作権と商標の情報**: Copyright (c) 2026 Akiva Miura

**追加のライセンス条項**: MIT License または Apache License 2.0 のいずれかを選択できます: https://github.com/akivajp/flexflyout#license

**開発元**: Akiva Miura
