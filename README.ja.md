# FlexFlyout

[English](README.md)

> **開発の初期段階です。** Windows 版から作っています。再生中の曲・音量・音量キーに合わせて表示し、設定は `%APPDATA%\flexflyout\config.toml` から読みます。タスクトレイのアイコンから、設定ファイルを開く・Windows の起動時に開始するかを切り替える・終了する、ができます。`flexflyout --demo` で見本の曲を表示できます。

柔軟に調整できる、クロスプラットフォームのメディアの Flyout です。音量を変えたときや新しい曲が始まったときに、再生中の曲の情報（ジャケット・曲名・アーティスト）を、再生位置、再生の操作ボタン（シャッフル・リピートを含む）、音量と一緒に表示し、しばらくすると消えます。バーをクリック・ドラッグすると再生位置や音量を変えられ、ピン留めすると表示したままにできます。

画面の大きさやモニターの構成は人によって違うので、表示のすべてを調整できるようにします。

- **大きさ**: 表示全体の拡大率
- **位置**: 9か所の基準点と、ピクセル単位のずらし幅
- **表示先のモニター**: メインのモニター、カーソルのある画面、番号で指定した画面
- **表示時間**と**不透明度**
- **OS 標準の音量表示**: 残して、その横に並べて表示するか、消すか

## 対応する環境

| 環境 | 状況 |
|---|---|
| Windows 11 | 最初に作る |
| Linux — KDE Plasma（Wayland） | 予定 |
| Linux — GNOME（Wayland） | 予定。FlexFlyout から送った内容を描くだけの、小さな GNOME Shell 拡張として作る |
| macOS | 将来 |

## 仕組み（予定）

- **再生中の曲の情報と操作**: Windows は System Media Transport Controls（SMTC）、Linux は MPRIS
- **音量**: Windows は Core Audio、Linux は PipeWire / PulseAudio
- **画面**: Rust と [egui](https://github.com/emilk/egui)
- **表示言語**: まず英語と日本語。ほかの言語も足しやすい、翻訳の仕組みの上に作る

## 作る理由

Windows 10 では、音量の表示に、再生中の曲と再生の操作ボタンも出ていました。Windows 11 では、この部分が無くなりました。代わりになる他社のソフトは、最近の Windows 11 で動かなくなったか、大きさや位置をほとんど調整できず、Linux のデスクトップで動くものもありません。FlexFlyout は、この2つの穴を埋めることを目指します。

## ライセンス

次のどちらかを選んで使えます。

- Apache License, Version 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- MIT ライセンス（[LICENSE-MIT](LICENSE-MIT)）
