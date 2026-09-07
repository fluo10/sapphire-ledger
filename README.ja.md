# sapphire-ledger（日本語）

> Language: [English](README.md) | **日本語**

ローカルファーストの複式簿記家計簿。データをプレーンテキストのまま永遠に生きる化石のように保ちます。

## コンセプト

- **TOMLをソース・オブ・トゥルースに** — すべてのデータはプレーンな`.toml`ファイルに保存され、どんなツールでも読み書きできます
- **キャッシュ不要** — `load_workspace`は毎回TOMLファイルを直接読み込みます。家庭規模ならそれで十分速いので、SQLiteもデータベースも不要です
- **複式簿記** — すべての取引は貸借がバランスしたポスティング（通貨ごとに借方＝貸方）の集合です
- **最初から多通貨対応** — ポスティングは通貨をを持ち、通貨をまたぐ取引にはインラインの為替レートを扱います
- **1レコード1ファイル** — 取引・アカウント・残高アサーション・為替レートログの各エントリはそれぞれ独立したファイルに置かれ、人間とAIによる共同編集でもgitのマージコンフリクトが起きにくいようにしています
- **人間とAIの共同編集** — AIエージェント（Claudeなど）がgitやSyncthing同期を介して同じレジャーの読み書き・作成・編集をすることを想定して設計されています

## プロジェクト構成

```
sapphire-ledger/
├── cli/                     # CLIバイナリ（sapphire-ledger）。stdio MCPサーバー同梱
├── desktop/                 # デスクトップGUI（egui）。まだスキャフォールド
├── server/                  # HTTP経由のセルフホストMCPサーバー。デバイス毎の認証（/rpc同期は未実装）
└── crates/
    ├── sapphire-ledger-core/  # データモデル、TOMLパーサ/シリアライザ、検証、書き込みパス
    └── sapphire-ledger-mcp/   # MCPサーバーロジック（ライブラリ。CLIとserver/が組み込んで使用）
```

## ステータス

データモデル、検証、書き込みパス、MCPサーバーは動作します。MCPクライアントを`sapphire-ledger mcp`に向けてstdio経由でエントリの読み書き・記録をしたり、`sapphire-ledger-server`インスタンスの`/mcp`に向けてHTTP経由で同じことができます（デバイス毎の認証付き。CLIと認証モデルは[`docs/design.md`](docs/design.md#mcp-server)参照）。`/rpc`同期は未実装なので、エージェントが書いた内容の修正はstillワークスペースのファイルを持つマシンで行う必要があります — このプロジェクトが存在意義とするレビューループはまだ閉じていません。デスクトップGUIはまだスキャフォールドですが、CLIは書き込みに対応しました：

```
sapphire-ledger account add Expenses:Food --type Expense
sapphire-ledger tx add --narration "groceries" \
  --posting "Expenses:Food 1200 JPY" --posting "Assets:Cash -1200 JPY"
sapphire-ledger tx list --account Expenses:Food
```

`assertion add`と`price add`も利用可能です。すべてMCPツールと同じ`core::ops`の薄いラッパーなので、誰が入力してもルールは同じです。

## ライセンス

このリポジトリは異なるライセンスのコンポーネントを含みます：

| コンポーネント | ライセンス |
|-----------|---------|
| `sapphire-ledger-core` | MIT OR Apache-2.0 |
| `sapphire-ledger-mcp` | MIT OR Apache-2.0 |
| `sapphire-ledger-cli` | MIT OR Apache-2.0 |
| `sapphire-ledger-desktop` | MIT OR Apache-2.0 |
| `sapphire-ledger-server` | MIT OR Apache-2.0 |

各コンポーネントのディレクトリにある`LICENSE-MIT` / `LICENSE-APACHE`ファイルに完全なライセンス文書があります。
