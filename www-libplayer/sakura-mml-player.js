import init, {
  MidiPlayer,
  compile_mml,
  load_soundfont,
} from './pkg/sakuramml_player.js';

const DEFAULT_CHUNK_SEC = 5;
const DEFAULT_BUFFER_AHEAD_SEC = 15;
const DEFAULT_SCHEDULE_LATENCY = 0.04;

export class SakuraPlayer {
  constructor(options = {}) {
    this.sampleRate = options.sampleRate || 44100;
    this.soundFontUrl = options.soundFontUrl || null;
    this.chunkSeconds = options.chunkSeconds || DEFAULT_CHUNK_SEC;
    this.bufferAheadSeconds = options.bufferAheadSeconds || DEFAULT_BUFFER_AHEAD_SEC;
    this.scheduleLatency = options.scheduleLatency || DEFAULT_SCHEDULE_LATENCY;

    this.wasmReady = false;
    this.soundFontLoaded = false;
    this.audioCtx = null;
    this.gainNode = null;
    this.player = null;

    this.midiBytes = null;
    this.midiInfo = null;
    this.duration = 0;
    this.playLength = null;
    this.pauseTime = 0;
    this.isPlaying = false;
    this.renderComplete = false;

    this.playStartAcTime = 0;
    this.playStartSec = 0;
    this.scheduledUpTo = 0;
    this.chunkNodes = [];
    this.pumpTimer = null;
    this.positionTimer = null;
    this.listeners = new Map();
  }

  async init() {
    if (!this.wasmReady) {
      await init();
      this.wasmReady = true;
      this.emit('init', {});
    }
    return this;
  }

  async compileMML(source) {
    await this.init();
    const result = compile_mml(source);
    const midi = result.bin;
    const log = result.log;
    this.emit('compile', { midi, log });
    return { midi, log };
  }

  async loadSoundFont(source = this.soundFontUrl) {
    await this.init();
    if (source == null) {
      throw new Error('SoundFont の URL またはバイト列を loadSoundFont() に指定してください。');
    }
    const data = await this.toUint8Array(source);
    try {
      load_soundfont(data);
    } catch (error) {
      if (!String(error?.message || error).includes('already loaded')) {
        throw error;
      }
    }
    this.soundFontLoaded = true;
    this.emit('soundfont', { byteLength: data.byteLength });
    return this;
  }

  async play(input = null) {
    await this.init();
    await this.ensureAudio();

    if (input != null) {
      if (typeof input === 'string') {
        const { midi } = await this.compileMML(input);
        this.loadMIDI(midi);
      } else {
        this.loadMIDI(await this.toUint8Array(input));
      }
      this.pauseTime = 0;
    } else if (!this.player) {
      throw new Error('再生する MIDI が読み込まれていません。');
    }

    if (this.audioCtx.state === 'suspended') {
      await this.audioCtx.resume();
    }

    this.stopScheduledChunks();
    this.player.seek_to(this.pauseTime);
    this.renderComplete = this.player.is_render_done();
    this.playStartSec = this.pauseTime;
    this.playStartAcTime = this.audioCtx.currentTime + this.scheduleLatency;
    this.scheduledUpTo = this.playStartAcTime;
    this.isPlaying = true;

    this.emit('play', { position: this.pauseTime, duration: this.duration });
    this.startPositionTimer();
    this.pumpChunks();
  }

  pause() {
    if (!this.isPlaying) return;
    this.pauseTime = this.getCurrentTime();
    this.stopScheduledChunks();
    this.isPlaying = false;
    this.stopTimers();
    this.emit('pause', { position: this.pauseTime });
  }

  stop() {
    this.stopScheduledChunks();
    this.isPlaying = false;
    this.pauseTime = 0;
    if (this.player) {
      this.player.seek_to(0);
    }
    this.stopTimers();
    this.emit('stop', { position: 0 });
  }

  setPosistion(seconds) {
    const next = this.clampPosition(seconds);
    this.pauseTime = next;
    if (this.player) {
      this.player.seek_to(next);
    }
    if (this.isPlaying) {
      this.play();
    } else {
      this.emit('position', { position: next, duration: this.duration });
    }
  }

  setPosition(seconds) {
    this.setPosistion(seconds);
  }

  setLength(seconds) {
    const value = Number(seconds);
    this.playLength = Number.isFinite(value) && value > 0 ? value : null;
    if (this.playLength != null && this.pauseTime > this.playLength) {
      this.setPosistion(this.playLength);
    }
    this.emit('length', { length: this.playLength });
  }

  onEvent(name, handler = null) {
    if (typeof name === 'function') {
      this.addListener('*', name);
      return () => this.removeListener('*', name);
    }
    if (typeof handler !== 'function') {
      throw new Error('onEvent にはイベント名と関数を指定してください。');
    }
    this.addListener(name, handler);
    return () => this.removeListener(name, handler);
  }

  setVolume(value) {
    const volume = Math.max(0, Math.min(1, Number(value)));
    if (this.gainNode) {
      this.gainNode.gain.value = volume;
    }
    this.emit('volume', { volume });
  }

  getCurrentTime() {
    if (this.isPlaying && this.audioCtx) {
      const elapsed = Math.max(0, this.audioCtx.currentTime - this.playStartAcTime);
      return this.clampPosition(this.playStartSec + elapsed);
    }
    return this.clampPosition(this.pauseTime);
  }

  getDuration() {
    return this.duration;
  }

  loadMIDI(bytes) {
    const data = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    this.midiBytes = data;
    this.player = new MidiPlayer(this.audioCtx?.sampleRate || this.sampleRate);
    this.midiInfo = JSON.parse(this.player.load(data));
    this.duration = this.player.get_duration();
    this.pauseTime = 0;
    this.renderComplete = false;
    this.emit('load', {
      duration: this.duration,
      notes: this.midiInfo.notes || [],
      texts: this.midiInfo.texts || [],
      beats: this.midiInfo.beats || [],
    });
    return this.midiInfo;
  }

  async ensureAudio() {
    if (this.audioCtx) return;
    const AudioContextClass = window.AudioContext || window.webkitAudioContext;
    if (!AudioContextClass) {
      throw new Error('このブラウザは Web Audio API に対応していません。');
    }
    this.audioCtx = new AudioContextClass({ sampleRate: this.sampleRate });
    this.sampleRate = this.audioCtx.sampleRate;
    this.gainNode = this.audioCtx.createGain();
    this.gainNode.gain.value = 0.8;
    this.gainNode.connect(this.audioCtx.destination);
  }

  pumpChunks() {
    if (!this.isPlaying || !this.player || !this.audioCtx) return;

    const current = this.getCurrentTime();
    const endAt = this.getPlaybackEnd();
    if (current >= endAt) {
      this.finishPlayback();
      return;
    }

    const limit = this.audioCtx.currentTime + this.bufferAheadSeconds;
    const chunkFrames = Math.max(1, Math.floor(this.sampleRate * this.chunkSeconds));

    while (!this.renderComplete && this.scheduledUpTo < limit) {
      const currentRenderSec = this.player.get_render_pos() / this.sampleRate;
      const remainSec = endAt - currentRenderSec;
      if (remainSec <= 0) {
        this.renderComplete = true;
        break;
      }

      const frames = Math.max(1, Math.min(chunkFrames, Math.ceil(remainSec * this.sampleRate)));
      const pcm = this.player.render_next(frames);
      if (pcm.length === 0) {
        this.renderComplete = true;
        break;
      }

      const source = this.createBufferSource(pcm);
      const entry = {
        source,
        startAcTime: this.scheduledUpTo,
        endAcTime: this.scheduledUpTo + source.buffer.duration,
      };
      this.chunkNodes.push(entry);

      source.onended = () => {
        this.chunkNodes = this.chunkNodes.filter((item) => item !== entry);
        if (this.isPlaying) {
          this.pumpChunks();
        }
      };

      source.start(this.scheduledUpTo);
      this.scheduledUpTo += source.buffer.duration;
      this.renderComplete = this.player.is_render_done();
    }

    this.schedulePump();
  }

  createBufferSource(pcm) {
    const frames = pcm.length / 2;
    const buffer = this.audioCtx.createBuffer(2, frames, this.sampleRate);
    const left = buffer.getChannelData(0);
    const right = buffer.getChannelData(1);
    for (let i = 0; i < frames; i++) {
      left[i] = pcm[i * 2];
      right[i] = pcm[i * 2 + 1];
    }

    const source = this.audioCtx.createBufferSource();
    source.buffer = buffer;
    source.connect(this.gainNode);
    return source;
  }

  finishPlayback() {
    this.stopScheduledChunks();
    this.isPlaying = false;
    this.pauseTime = this.getPlaybackEnd();
    this.stopTimers();
    this.emit('ended', { position: this.pauseTime, duration: this.duration });
  }

  stopScheduledChunks() {
    for (const { source } of this.chunkNodes) {
      try {
        source.stop();
      } catch (_) {
        // すでに終了済みの source は無視します。
      }
      source.disconnect();
    }
    this.chunkNodes = [];
    this.renderComplete = false;
    if (this.pumpTimer) {
      clearTimeout(this.pumpTimer);
      this.pumpTimer = null;
    }
  }

  schedulePump() {
    if (this.pumpTimer) return;
    this.pumpTimer = setTimeout(() => {
      this.pumpTimer = null;
      this.pumpChunks();
    }, 250);
  }

  startPositionTimer() {
    if (this.positionTimer) return;
    this.positionTimer = setInterval(() => {
      const position = this.getCurrentTime();
      this.emit('position', { position, duration: this.duration });
      if (position >= this.getPlaybackEnd()) {
        this.finishPlayback();
      }
    }, 100);
  }

  stopTimers() {
    if (this.pumpTimer) {
      clearTimeout(this.pumpTimer);
      this.pumpTimer = null;
    }
    if (this.positionTimer) {
      clearInterval(this.positionTimer);
      this.positionTimer = null;
    }
  }

  getPlaybackEnd() {
    return this.playLength == null ? this.duration : Math.min(this.duration, this.playLength);
  }

  clampPosition(seconds) {
    const value = Number(seconds);
    if (!Number.isFinite(value)) return 0;
    return Math.max(0, Math.min(value, this.getPlaybackEnd() || this.duration || value));
  }

  addListener(name, handler) {
    if (!this.listeners.has(name)) {
      this.listeners.set(name, new Set());
    }
    this.listeners.get(name).add(handler);
  }

  removeListener(name, handler) {
    this.listeners.get(name)?.delete(handler);
  }

  emit(name, detail) {
    const event = { type: name, detail, player: this };
    for (const handler of this.listeners.get(name) || []) {
      handler(event);
    }
    for (const handler of this.listeners.get('*') || []) {
      handler(event);
    }
  }

  async toUint8Array(source) {
    if (source instanceof Uint8Array) {
      return source;
    }
    if (source instanceof ArrayBuffer) {
      return new Uint8Array(source);
    }
    if (ArrayBuffer.isView(source)) {
      return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
    }
    if (source instanceof Blob) {
      return new Uint8Array(await source.arrayBuffer());
    }
    const response = await fetch(source);
    if (!response.ok) {
      throw new Error(`読み込みに失敗しました: ${response.status} ${response.statusText}`);
    }
    return new Uint8Array(await response.arrayBuffer());
  }
}

export async function createSakuraPlayer(options = {}) {
  const player = new SakuraPlayer(options);
  await player.init();
  return player;
}

export const createSakuraMmlSoundPlayer = createSakuraPlayer;
export { SakuraPlayer as SakuraMmlSoundPlayer };
