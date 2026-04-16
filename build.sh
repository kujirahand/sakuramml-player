#!/bin/bash
# ─────────────────────────────────────────────────────────
# サクラ MIDI プレイヤー — ビルドスクリプト
# ─────────────────────────────────────────────────────────
set -e

echo "🌸 サクラ MIDI プレイヤー ビルド開始"
echo ""

# wasm-pack が必要
if ! command -v wasm-pack &> /dev/null; then
  echo "❌ wasm-pack が見つかりません。インストールしてください:"
  echo "   curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh"
  exit 1
fi

# Rust/Wasm をビルドして www/pkg/ に出力
echo "📦 wasm-pack build --target web --out-dir www/pkg ..."
wasm-pack build --target web --out-dir www/pkg

echo ""
echo "✅ ビルド完了! www/pkg/ に Wasm モジュールが生成されました。"
echo ""
echo "ブラウザで開くには:"
echo "  cd www && python3 -m http.server 8080"
echo "  → http://localhost:8080"
echo ""
