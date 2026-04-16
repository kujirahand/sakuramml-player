/**
 * サクラ MIDI プレイヤー — メイン JS
 *
 * ## ストリーミング再生の仕組み
 *
 * 1. player.load() で MIDI を解析しノートリストを取得
 * 2. 再生開始時に player.seek_to(t) でカーソルをセット
 * 3. pumpChunks() が 10 秒ずつ player.render_next() を呼んで
 *    AudioBufferSourceNode を AudioContext 時刻でスケジュール
 * 4. 各 chunk の再生終了 (onended) が次の pumpChunks() を起動
 *    → 常に BUFFER_AHEAD_SEC 秒分先読みを維持
 * 5. シークは stopAllChunks() → seek_to() → pumpChunks() のリセット
 */

import init, { MidiPlayer, load_soundfont, compile_mml, encoding_to_utf8 } from './pkg/sakuramml_player.js?v=2';

// ─────────────────────────────────────────────────────────
// 定数
// ─────────────────────────────────────────────────────────

const SAMPLE_RATE = 44100;
const CHUNK_SEC = 10;                     // 1 チャンクの長さ (秒)
const CHUNK_FRAMES = SAMPLE_RATE * CHUNK_SEC; // = 441 000 サンプル
const BUFFER_AHEAD_SEC = 25;                     // 先読みバッファ量 (秒)
const SCHEDULE_LATENCY = 0.04;                   // 再生開始までの最小遅延 (秒)

const PIANO_W = 64;
const RULER_H = 24;
const NOTE_MIN_W = 2;

const CHANNEL_COLORS = [
  '#6366f1', '#8b5cf6', '#ec4899', '#ef4444',
  '#f97316', '#eab308', '#22c55e', '#14b8a6',
  '#06b6d4', '#3b82f6', '#a855f7', '#f43f5e',
  '#84cc16', '#f59e0b', '#10b981', '#0ea5e9',
];
const BLACK_SEMITONES = new Set([1, 3, 6, 8, 10]);

// ─────────────────────────────────────────────────────────
// プレイバック状態
// ─────────────────────────────────────────────────────────

let player = null;
let audioCtx = null;
let gainNode = null;

// --- ストリーミング状態 ---
let isPlaying = false;
let playStartAcTime = 0;    // AudioContext 時刻: この時点が playStartSec 秒
let playStartSec = 0;    // 再生開始時の曲位置 (秒)
let pauseTime = 0;    // 一時停止位置 (秒)

/** スケジュール済みチャンクノードの配列 { src, endAcTime } */
let chunkNodes = [];
/** 次のチャンクをスケジュールする AudioContext 時刻 */
let scheduledUpTo = 0;
/** Rust側が全サンプルを出力済みか */
let renderComplete = false;

// --- ピアノロール状態 ---
let notes = [];
let beats = [];
let duration = 0;
let noteHeight = 8;
let pps = 120;   // pixels per second
let scrollX = 0;
let scrollNote = 0;
let logW = 0, logH = 0;
let animId = null;
let mmlDirty = false;

// ─────────────────────────────────────────────────────────
// DOM
// ─────────────────────────────────────────────────────────

const canvas = document.getElementById('piano-roll');
const ctx2d = canvas.getContext('2d');
const fileInput = document.getElementById('file-input');
const playBtn = document.getElementById('play-btn');
const stopBtn = document.getElementById('stop-btn');
const seekBar = document.getElementById('seek-bar');
const seekFill = document.getElementById('seek-fill');
const curTimeEl = document.getElementById('current-time');
const totTimeEl = document.getElementById('total-time');
const statusEl = document.getElementById('status');
const zoomInBtn = document.getElementById('zoom-in');
const zoomOutBtn = document.getElementById('zoom-out');
const zoomLabelEl = document.getElementById('zoom-label');
const volumeSlider = document.getElementById('volume');
const dropOverlay = document.getElementById('drop-overlay');
const rollContainer = document.getElementById('roll-container');

// MML エディタ
const mmlInput = document.getElementById('mml-input');

// ─────────────────────────────────────────────────────────
// 初期化
// ─────────────────────────────────────────────────────────

async function main() {
  setStatus('Wasm 初期化中…');
  try {
    await init();

    setStatus('サウンドフォントをロード中…');
    const response = await fetch('./fonts/TimGM6mb.sf2');
    if (!response.ok) {
      throw new Error(`Failed to fetch soundfont: ${response.statusText}`);
    }
    const sf2Data = await response.arrayBuffer();
    load_soundfont(new Uint8Array(sf2Data));

    player = new MidiPlayer(SAMPLE_RATE);
    setStatus('ファイルをキャンバスにドロップしてください', 'ok');
  } catch (e) {
    setStatus('初期化失敗: ' + e.message, 'err');
    console.error(e);
    return;
  }
  setupEvents();
  resizeCanvas();
  drawFrame(0);
}

// ─────────────────────────────────────────────────────────
// イベント設定
// ─────────────────────────────────────────────────────────

function setupEvents() {
  fileInput.addEventListener('change', e => {
    const f = e.target.files[0];
    console.log('[FileInput] File selected:', f ? f.name : 'null');
    if (f) loadFile(f);
  });
  playBtn.addEventListener('click', togglePlay);
  stopBtn.addEventListener('click', stopPlayback);
  zoomInBtn.addEventListener('click', () => zoomAround(1.4));
  zoomOutBtn.addEventListener('click', () => zoomAround(1 / 1.4));

  seekBar.addEventListener('input', onSeekInput);
  seekBar.addEventListener('change', onSeekChange);

  volumeSlider.addEventListener('input', () => {
    if (gainNode) gainNode.gain.value = +volumeSlider.value;
  });

  canvas.addEventListener('wheel', onWheel, { passive: false });

  let isDragging = false;
  let dragStartX = 0;
  let dragStartY = 0;
  let dragStartScrollNote = 0;
  let dragStartScrollX = 0;

  canvas.addEventListener('mousedown', e => {
    isDragging = true;
    dragStartX = e.clientX;
    dragStartY = e.clientY;
    dragStartScrollNote = scrollNote;
    dragStartScrollX = scrollX;
  });

  window.addEventListener('mousemove', e => {
    if (!isDragging) return;
    const dy = e.clientY - dragStartY;
    const dx = e.clientX - dragStartX;

    // Y方向のドラッグ（上下） -> dyが負（上ドラッグ）の時、scrollNoteを減らして鍵盤を上に移動
    scrollNote = Math.max(0, Math.min(115, dragStartScrollNote + dy / noteHeight));
    // X方向のドラッグ（左右） -> dxが負（左ドラッグ）の時、scrollXを増やしてグリッドを左に移動
    scrollX = Math.max(0, dragStartScrollX - dx);

    if (!isPlaying) drawFrame(getCurrentTime());
  });

  window.addEventListener('mouseup', () => {
    isDragging = false;
  });

  rollContainer.addEventListener('dragover', e => {
    e.preventDefault(); dropOverlay.classList.add('active');
  });
  rollContainer.addEventListener('dragleave', e => {
    if (!rollContainer.contains(e.relatedTarget)) dropOverlay.classList.remove('active');
  });
  rollContainer.addEventListener('drop', e => {
    e.preventDefault(); dropOverlay.classList.remove('active');
    const f = e.dataTransfer.files[0];
    console.log('[DropEvent] Dropped file:', f ? f.name : 'null');
    if (f) {
      if (/\.(midi?|mml)$/i.test(f.name)) {
        console.log('[DropEvent] File matched extension. Calling loadFile');
        loadFile(f);
      } else {
        console.log('[DropEvent] File extension did not match:', f.name);
      }
    }
  });

  window.addEventListener('resize', () => { resizeCanvas(); drawFrame(getCurrentTime()); });
  window.addEventListener('keydown', e => {
    // text area focus 時はスペースキー再生を無効化
    if (e.code === 'Space' && e.target === document.body) { e.preventDefault(); togglePlay(); }
  });

  mmlInput.addEventListener('input', () => {
    mmlDirty = true;
    if (mmlInput.value.trim().length > 0) {
      playBtn.disabled = false;
    } else if (!notes.length) {
      playBtn.disabled = true;
    }
  });

  if (mmlInput.value.trim().length > 0) {
    playBtn.disabled = false;
  }
}

// ─────────────────────────────────────────────────────────
// MML コンパイル
// ─────────────────────────────────────────────────────────

function compileAndPlayMml(mml) {
  showLoading(true, 'MMLをコンパイル中…');
  stopPlayback();

  setTimeout(() => {
    try {
      const bytes = compile_mml(mml);
      if (bytes && bytes.length > 0) {
        loadMidiBytes(bytes, 'MML');
      } else {
        throw new Error("コンパイルに失敗しました。");
      }
    } catch (e) {
      showLoading(false);
      setStatus('コンパイルエラー: ' + (e.message ?? e), 'err');
      console.error(e);
    }
  }, 10);
}

// ─────────────────────────────────────────────────────────
// MIDIファイル読み込み
// ─────────────────────────────────────────────────────────

async function loadFile(file) {
  console.log('[loadFile] Start loading file:', file.name);
  const isMml = /\.mml$/i.test(file.name);
  console.log('[loadFile] Is MML format?', isMml);
  showLoading(true, isMml ? 'MMLをコンパイル中…' : 'MIDIを解析中…');
  setStatus(`読み込み中: ${file.name}`);
  stopPlayback();

  try {
    if (isMml) {
      console.log('[loadFile] Reading MML file as ArrayBuffer for encoding_to_utf8');
      const fileBytes = new Uint8Array(await file.arrayBuffer());
      const text = encoding_to_utf8(fileBytes);
      console.log('[loadFile] Text read length:', text.length);
      mmlInput.value = text;
      mmlDirty = false;
      console.log('[loadFile] Calling compile_mml');
      const bytes = compile_mml(text);
      console.log('[loadFile] compile_mml generated bytes length:', bytes ? bytes.length : 0);
      if (bytes && bytes.length > 0) {
        console.log('[loadFile] Calling loadMidiBytes');
        loadMidiBytes(bytes, file.name);
      } else {
         console.warn('[loadFile] compile_mml failed or retured empty');
        throw new Error("コンパイルに失敗しました。");
      }
    } else {
      console.log('[loadFile] Reading MIDI file as ArrayBuffer');
      const bytes = new Uint8Array(await file.arrayBuffer());
      console.log('[loadFile] ArrayBuffer read length:', bytes.length);
      loadMidiBytes(bytes, file.name);
    }
  } catch (e) {
    showLoading(false);
    setStatus('エラー: ' + (e.message ?? e), 'err');
    console.error('[loadFile] Error:', e);
  }
}

function loadMidiBytes(bytes, title) {
  console.log('[loadMidiBytes] title:', title, 'bytes length:', bytes.length);
  try {
    // Rust/Wasm で MIDI 解析 & イベントリスト構築
    console.log('[loadMidiBytes] Calling player.load()');
    const json = player.load(bytes);
    console.log('[loadMidiBytes] player.load() success');
    const data = JSON.parse(json);
    notes = data.notes;
    beats = data.beats;
    duration = player.get_duration();

    totTimeEl.textContent = fmtTime(duration);
    seekBar.max = duration;
    seekBar.value = 0;
    updateSeekFill(0);

    // AudioContext が未作成なら生成
    if (!audioCtx) {
      audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });
      gainNode = audioCtx.createGain();
      gainNode.gain.value = +volumeSlider.value;
      gainNode.connect(audioCtx.destination);
    }

    setNotesViewport();
    showLoading(false);

    const totalSec = player.get_total_samples() / SAMPLE_RATE;
    const chunkCount = Math.ceil(totalSec / CHUNK_SEC);
    setStatus(
      `${title}  |  ${notes.length} 音符  |  ${fmtTime(duration)}` +
      `  |  チャンク数: ${chunkCount} (各 ${CHUNK_SEC}s)`,
      'ok'
    );

    playBtn.disabled = false;
    stopBtn.disabled = false;
    drawFrame(0);

    // Auto Play (optional, maybe nice for MML input)
    if (title === 'MML' || /\.mml$/i.test(title)) {
      startPlayback(0);
    }

  } catch (e) {
    showLoading(false);
    setStatus('エラー: ' + (e.message ?? e), 'err');
    console.error(e);
  }
}

// ─────────────────────────────────────────────────────────
// ストリーミング再生制御
// ─────────────────────────────────────────────────────────

/** 現在の再生位置 (秒) */
function getCurrentTime() {
  if (isPlaying && audioCtx) {
    const elapsed = Math.max(0, audioCtx.currentTime - playStartAcTime);
    return Math.min(duration, playStartSec + elapsed);
  }
  return pauseTime;
}

function togglePlay() {
  if (isPlaying) {
    pausePlayback();
    return;
  }

  const mml = mmlInput.value.trim();
  if (mml && (mmlDirty || !notes.length)) {
    mmlDirty = false;
    compileAndPlayMml(mml);
  } else if (notes.length) {
    startPlayback(pauseTime);
  }
}

/**
 * fromSec 秒からストリーミング再生を開始。
 * 既存のチャンクノードをすべて停止してから再スケジュールする。
 */
function startPlayback(fromSec = 0) {
  if (!audioCtx) return;
  if (audioCtx.state === 'suspended') audioCtx.resume();

  stopAllChunks();

  // Rust 側のカーソルをシーク位置へ
  player.seek_to(fromSec);
  renderComplete = player.is_render_done();

  playStartSec = fromSec;
  playStartAcTime = audioCtx.currentTime + SCHEDULE_LATENCY;
  scheduledUpTo = playStartAcTime;

  isPlaying = true;
  playBtn.textContent = '⏸';
  playBtn.title = '一時停止';

  pumpChunks();   // 先読みバッファを満たす
  animate();
}

/** 一時停止 */
function pausePlayback() {
  if (!isPlaying) return;
  pauseTime = getCurrentTime();
  stopAllChunks();
  isPlaying = false;
  cancelAnimationFrame(animId);
  playBtn.textContent = '▶';
  playBtn.title = '再生';
  drawFrame(pauseTime);
}

/** 停止 (先頭に戻る) */
function stopPlayback() {
  pausePlayback();
  pauseTime = 0;
  seekBar.value = 0;
  updateSeekFill(0);
  curTimeEl.textContent = fmtTime(0);
  drawFrame(0);
}

/** スケジュール済みの全 AudioBufferSourceNode を即停止して解放する */
function stopAllChunks() {
  for (const { src } of chunkNodes) {
    try { src.stop(); } catch (_) { }
    src.disconnect();
  }
  chunkNodes = [];
  renderComplete = false;
}

/**
 * 先読みバッファを BUFFER_AHEAD_SEC 秒分維持するようにチャンクを追加する。
 * 再生開始時・各チャンク再生終了時 (onended) に呼ばれる。
 */
function pumpChunks() {
  if (!isPlaying) return;

  // 全チャンクを再生し次が無ければ停止
  if (renderComplete && chunkNodes.length === 0) {
    stopPlayback();
    return;
  }

  const limit = audioCtx.currentTime + BUFFER_AHEAD_SEC;

  while (!renderComplete && scheduledUpTo < limit) {
    // 次の 10 秒分を Rust でレンダリング
    const pcm = player.render_next(CHUNK_FRAMES);
    if (pcm.length === 0) { renderComplete = true; break; }

    // AudioBuffer に変換 (インターリーブされたステレオを分離)
    const numFrames = pcm.length / 2;
    const buf = audioCtx.createBuffer(2, numFrames, SAMPLE_RATE);
    const leftData = buf.getChannelData(0);
    const rightData = buf.getChannelData(1);
    for (let i = 0; i < numFrames; i++) {
      leftData[i] = pcm[i * 2];
      rightData[i] = pcm[i * 2 + 1];
    }

    // AudioBufferSourceNode をスケジュール
    const src = audioCtx.createBufferSource();
    src.buffer = buf;
    src.connect(gainNode);
    src.start(scheduledUpTo);

    const entry = { src, startAcTime: scheduledUpTo, endAcTime: scheduledUpTo + buf.duration };
    chunkNodes.push(entry);
    scheduledUpTo += buf.duration;

    renderComplete = player.is_render_done();

    // チャンク再生終了時に次のチャンクをポンプ
    src.onended = () => {
      chunkNodes = chunkNodes.filter(c => c !== entry);
      pumpChunks();
    };
  }
}

// ─────────────────────────────────────────────────────────
// シーク
// ─────────────────────────────────────────────────────────

function onSeekInput(e) {
  const t = +e.target.value;
  curTimeEl.textContent = fmtTime(t);
  updateSeekFill(t);
  if (!isPlaying) { pauseTime = t; drawFrame(t); }
}

function onSeekChange(e) {
  const t = +e.target.value;
  if (isPlaying) {
    // 再生中はシーク位置から再スタート
    startPlayback(t);
  } else {
    pauseTime = t;
    drawFrame(t);
  }
}

// ─────────────────────────────────────────────────────────
// ズーム / スクロール
// ─────────────────────────────────────────────────────────

function zoomAround(factor) {
  const ct = getCurrentTime();
  pps = Math.max(20, Math.min(2000, pps * factor));
  scrollX = Math.max(0, pps * ct - (logW - PIANO_W) * 0.3);
  zoomLabelEl.textContent = Math.round(pps) + 'px/s';
  drawFrame(ct);
}

function onWheel(e) {
  e.preventDefault();
  const ct = getCurrentTime();
  if (e.ctrlKey || e.metaKey) {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const timeAtMouse = (mx - PIANO_W + scrollX) / pps;
    pps = Math.max(20, Math.min(2000, pps * (e.deltaY < 0 ? 1.12 : 0.89)));
    scrollX = Math.max(0, pps * timeAtMouse - (mx - PIANO_W));
    zoomLabelEl.textContent = Math.round(pps) + 'px/s';
  } else if (e.shiftKey) {
    scrollX = Math.max(0, scrollX + e.deltaY);
  } else {
    scrollNote = Math.max(0, Math.min(115, scrollNote + (e.deltaY > 0 ? 3 : -3)));
  }
  drawFrame(ct);
}

// ─────────────────────────────────────────────────────────
// アニメーションループ
// ─────────────────────────────────────────────────────────

function animate() {
  if (!isPlaying) return;

  const ct = getCurrentTime();
  seekBar.value = ct;
  updateSeekFill(ct);
  curTimeEl.textContent = fmtTime(ct);

  // 再生ヘッドが右端 70% を超えたら自動スクロール
  const headX = PIANO_W + ct * pps - scrollX;
  const rollW = logW - PIANO_W;
  if (headX > PIANO_W + rollW * 0.72) {
    scrollX = Math.max(0, ct * pps - rollW * 0.28);
  }

  // バッファが不足していれば追加スケジュール
  if (!renderComplete && scheduledUpTo < audioCtx.currentTime + BUFFER_AHEAD_SEC) {
    pumpChunks();
  }

  drawFrame(ct);
  animId = requestAnimationFrame(animate);
}

// ─────────────────────────────────────────────────────────
// Canvas リサイズ
// ─────────────────────────────────────────────────────────

function resizeCanvas() {
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  logW = rect.width;
  logH = rect.height;
  canvas.width = logW * dpr;
  canvas.height = logH * dpr;
  ctx2d.scale(dpr, dpr);
}

// ─────────────────────────────────────────────────────────
// ピアノロール描画
// ─────────────────────────────────────────────────────────

function drawFrame(ct) {
  if (logW === 0 || logH === 0) return;
  const W = logW, H = logH;
  ctx2d.clearRect(0, 0, W, H);
  ctx2d.fillStyle = '#07071a';
  ctx2d.fillRect(0, 0, W, H);

  const rollH = H - RULER_H;
  const visNotes = Math.floor(rollH / noteHeight);

  if (notes.length === 0) { drawEmpty(W, H); return; }

  drawGrid(W, H, visNotes);
  drawBeatLines(W, H);
  drawPianoKeys(H, visNotes);
  drawTimeRuler(W);
  drawNotes(W, H, visNotes, ct);
  drawPlayhead(H, ct);
}

function drawEmpty(W, H) {
  ctx2d.fillStyle = 'rgba(255,255,255,0.12)';
  ctx2d.font = '500 16px Inter, sans-serif';
  ctx2d.textAlign = 'center';
  ctx2d.textBaseline = 'middle';
  ctx2d.fillText('🌸  MIDI / MML ファイルをドロップするか「開く」で選択', W / 2, H / 2);
}

function drawGrid(W, H, visNotes) {
  for (let i = 0; i < visNotes; i++) {
    const note = scrollNote + i;
    const y = noteToY(note, H);
    if (isBlack(note)) {
      ctx2d.fillStyle = 'rgba(0,0,0,0.22)';
      ctx2d.fillRect(PIANO_W, y, W - PIANO_W, noteHeight);
    }
    if (note % 12 === 11) {
      ctx2d.strokeStyle = 'rgba(80,80,130,0.35)';
      ctx2d.lineWidth = 0.5;
      ctx2d.beginPath();
      ctx2d.moveTo(PIANO_W, y); ctx2d.lineTo(W, y);
      ctx2d.stroke();
    }
  }
}

function drawBeatLines(W, H) {
  const startSec = scrollX / pps;
  const endSec = (scrollX + W - PIANO_W) / pps;

  for (const b of beats) {
    if (b.time < startSec || b.time > endSec) continue;
    const x = PIANO_W + b.time * pps - scrollX;
    if (x < PIANO_W) continue;

    if (b.is_measure) {
      // 小節の先頭（太めの線）
      ctx2d.strokeStyle = 'rgba(120, 130, 200, 0.5)';
      ctx2d.lineWidth = 1.5;
    } else {
      // 拍の区切り（細い線）
      ctx2d.strokeStyle = 'rgba(90, 100, 160, 0.35)'; // 色を濃く、不透明度を上げる
      ctx2d.lineWidth = 1.0;  // 0.5 -> 1.0 に変更してぼやけを防ぐ
    }
    ctx2d.beginPath();
    ctx2d.moveTo(x, RULER_H);
    ctx2d.lineTo(x, H);
    ctx2d.stroke();
  }
}

function drawPianoKeys(H, visNotes) {
  for (let i = 0; i < visNotes; i++) {
    const note = scrollNote + i;
    const y = noteToY(note, H);
    const blk = isBlack(note);
    ctx2d.fillStyle = blk ? '#0b0b20' : '#131328';
    ctx2d.fillRect(0, y, PIANO_W, noteHeight);
    if (!blk) {
      ctx2d.fillStyle = '#1c1c3a';
      ctx2d.fillRect(PIANO_W - 18, y + 1, 18, noteHeight - 2);
    }
    ctx2d.strokeStyle = 'rgba(20,20,50,0.9)';
    ctx2d.lineWidth = 0.5;
    ctx2d.beginPath(); ctx2d.moveTo(0, y); ctx2d.lineTo(PIANO_W, y); ctx2d.stroke();
    if (note % 12 === 0 && noteHeight >= 7) {
      ctx2d.fillStyle = '#5060a0';
      ctx2d.font = `${Math.min(noteHeight - 2, 10)}px Inter, sans-serif`;
      ctx2d.textAlign = 'left'; ctx2d.textBaseline = 'middle';
      ctx2d.fillText('C' + (Math.floor(note / 12) - 1), 3, y + noteHeight / 2);
    }
  }
  ctx2d.strokeStyle = 'rgba(60,60,120,0.6)';
  ctx2d.lineWidth = 1;
  ctx2d.beginPath(); ctx2d.moveTo(PIANO_W, RULER_H); ctx2d.lineTo(PIANO_W, H); ctx2d.stroke();
}

function drawTimeRuler(W) {
  ctx2d.fillStyle = '#0d0d22';
  ctx2d.fillRect(0, 0, W, RULER_H);
  ctx2d.strokeStyle = 'rgba(60,60,110,0.6)'; ctx2d.lineWidth = 1;
  ctx2d.beginPath(); ctx2d.moveTo(0, RULER_H); ctx2d.lineTo(W, RULER_H); ctx2d.stroke();

  let interval = 1;
  if (pps < 30) interval = 30;
  else if (pps < 60) interval = 10;
  else if (pps < 120) interval = 5;
  else if (pps < 300) interval = 2;
  else if (pps > 600) interval = 0.5;

  const startSec = scrollX / pps;
  const endSec = (scrollX + W - PIANO_W) / pps;
  const first = Math.ceil(startSec / interval) * interval;

  ctx2d.fillStyle = '#6070a0'; ctx2d.font = '10px Inter, monospace';
  ctx2d.textAlign = 'center'; ctx2d.textBaseline = 'top';

  for (let t = first; t <= endSec + interval * 0.01; t += interval) {
    const x = PIANO_W + t * pps - scrollX;
    if (x < PIANO_W) continue;
    ctx2d.strokeStyle = 'rgba(60,60,120,0.5)'; ctx2d.lineWidth = 0.5;
    ctx2d.beginPath(); ctx2d.moveTo(x, RULER_H - 7); ctx2d.lineTo(x, RULER_H); ctx2d.stroke();
    ctx2d.fillStyle = '#6070a0';
    ctx2d.fillText(fmtTime(t), x, 4);
  }
}

function drawNotes(W, H, visNotes, ct) {
  const startSec = scrollX / pps;
  const endSec = (scrollX + W - PIANO_W) / pps;

  for (const n of notes) {
    if (n.time + n.dur < startSec || n.time > endSec) continue;
    if (n.note < scrollNote || n.note >= scrollNote + visNotes) continue;

    const x = PIANO_W + n.time * pps - scrollX;
    const w = Math.max(NOTE_MIN_W, n.dur * pps);
    const y = noteToY(n.note, H);
    const h = noteHeight - 1;
    const col = CHANNEL_COLORS[n.ch % CHANNEL_COLORS.length];
    const active = ct >= n.time && ct < n.time + n.dur + 0.01;

    if (active) {
      ctx2d.fillStyle = col + '28';
      ctx2d.fillRect(0, y + 0.5, PIANO_W - 1, h);
    }
    ctx2d.save();
    ctx2d.globalAlpha = active ? 1.0 : 0.82;
    ctx2d.fillStyle = col;
    ctx2d.strokeStyle = active ? '#ffffffaa' : col + '60';
    ctx2d.lineWidth = 0.5;
    roundRect(ctx2d, x, y + 0.5, w, h, 2);
    ctx2d.fill(); ctx2d.stroke();
    ctx2d.restore();
  }
}

function drawPlayhead(H, ct) {
  const x = PIANO_W + ct * pps - scrollX;
  if (x < PIANO_W - 2 || x > logW + 2) return;
  const g = ctx2d.createLinearGradient(x - 5, 0, x + 5, 0);
  g.addColorStop(0, 'transparent');
  g.addColorStop(0.5, 'rgba(167,139,250,0.35)');
  g.addColorStop(1, 'transparent');
  ctx2d.fillStyle = g; ctx2d.fillRect(x - 6, 0, 12, H);
  ctx2d.strokeStyle = '#c4b5fd'; ctx2d.lineWidth = 1.5;
  ctx2d.beginPath(); ctx2d.moveTo(x, 0); ctx2d.lineTo(x, H); ctx2d.stroke();
  ctx2d.fillStyle = '#c4b5fd';
  ctx2d.beginPath(); ctx2d.moveTo(x - 6, 0); ctx2d.lineTo(x + 6, 0); ctx2d.lineTo(x, 10); ctx2d.closePath(); ctx2d.fill();
}

// ─────────────────────────────────────────────────────────
// ユーティリティ
// ─────────────────────────────────────────────────────────

function noteToY(note, H) {
  const visNotes = Math.floor((H - RULER_H) / noteHeight);
  return RULER_H + (visNotes - (note - scrollNote) - 1) * noteHeight;
}
function isBlack(note) { return BLACK_SEMITONES.has(note % 12); }

function roundRect(ctx, x, y, w, h, r) {
  if (ctx.roundRect) { ctx.beginPath(); ctx.roundRect(x, y, w, h, r); return; }
  ctx.beginPath();
  ctx.moveTo(x + r, y); ctx.arcTo(x + w, y, x + w, y + h, r); ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r); ctx.arcTo(x, y, x + w, y, r); ctx.closePath();
}

function fmtTime(sec) {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const m = Math.floor(sec / 60), s = Math.floor(sec % 60);
  return `${m}:${String(s).padStart(2, '0')}`;
}

function updateSeekFill(t) {
  seekFill.style.width = (duration > 0 ? t / duration * 100 : 0).toFixed(2) + '%';
}

function setStatus(msg, cls = '') {
  statusEl.textContent = msg;
  statusEl.className = 'status-badge' + (cls ? ' ' + cls : '');
}

let loadingEl = null;
function showLoading(show, msg = '') {
  if (show) {
    if (!loadingEl) {
      loadingEl = document.createElement('div');
      loadingEl.className = 'loading-overlay';
      loadingEl.innerHTML = `<div class="spinner"></div><p class="loading-text"></p>`;
      rollContainer.appendChild(loadingEl);
    }
    loadingEl.querySelector('.loading-text').textContent = msg;
    loadingEl.style.display = 'flex';
  } else if (loadingEl) {
    loadingEl.style.display = 'none';
  }
}

function setNotesViewport() {
  if (!notes.length) return;
  const minNote = Math.max(0, notes.reduce((m, n) => Math.min(m, n.note), 127) - 4);
  const maxNote = Math.min(127, notes.reduce((m, n) => Math.max(m, n.note), 0) + 4);
  const range = maxNote - minNote + 1;
  noteHeight = Math.max(4, Math.min(18, Math.floor((logH - RULER_H) / range)));
  scrollNote = minNote;
  scrollX = 0;
  seekBar.value = 0;
  updateSeekFill(0);
}

// ─────────────────────────────────────────────────────────
// 起動
// ─────────────────────────────────────────────────────────

main().catch(console.error);
