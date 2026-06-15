import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, 'sakura-mml-player.js'), 'utf8');
const sample = readFileSync(join(here, '../www-libplayer-sample/index.html'), 'utf8');
const packageJson = JSON.parse(readFileSync(join(here, 'package.json'), 'utf8'));

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

const requiredPackageFiles = [
  'pkg/sakuramml_player.js',
  'pkg/sakuramml_player_bg.wasm',
  'pkg/sakuramml_player.d.ts',
  'pkg/sakuramml_player_bg.wasm.d.ts',
  'pkg/package.json',
  'pkg/README.md',
];

for (const file of requiredPackageFiles) {
  if (!packageJson.files.includes(file)) {
    throw new Error(`package.json の files に ${file} がありません。`);
  }
  if (!existsSync(join(here, file))) {
    throw new Error(`${file} が見つかりません。`);
  }
}

if (existsSync(join(here, 'pkg/.gitignore'))) {
  throw new Error('pkg/.gitignore が残っています。npm pack で pkg の中身が除外される可能性があります。');
}

console.log('www-libplayer の公開 API とサンプル HTML を確認しました。');
