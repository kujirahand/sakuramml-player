import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, 'sakura-mml-player.js'), 'utf8');
const sample = readFileSync(join(here, '../www-libplayer-sample/index.html'), 'utf8');

const requiredMethods = [
  'compileMML',
  'loadSoundFont',
  'play',
  'stop',
  'pause',
  'setPosistion',
  'setLength',
  'onEvent',
];

for (const method of requiredMethods) {
  if (!source.includes(`${method}(`)) {
    throw new Error(`SakuraPlayer.${method} が見つかりません。`);
  }
}

if (!source.includes('export class SakuraPlayer')) {
  throw new Error('SakuraPlayer クラスが公開されていません。');
}

if (!sample.includes("from '../www-libplayer/sakura-mml-player.js'")) {
  throw new Error('サンプル HTML がライブラリを読み込んでいません。');
}

if (!source.includes("from './pkg/sakuramml_player.js'")) {
  throw new Error('ライブラリが www-libplayer/pkg の Wasm バンドルを参照していません。');
}

if (source.includes('./fonts/')) {
  throw new Error('ライブラリが同梱 SoundFont を参照しています。');
}

console.log('www-libplayer の公開 API とサンプル HTML を確認しました。');
