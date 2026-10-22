// Plays sentences back one at a time. Each sentence is rendered by the Rust
// side (cached on disk); the next two are requested while the current one
// plays, so playback starts fast and rarely stalls.
import { api } from "./api";
import type { Sentence } from "./sentences";

export interface Voice {
  voice: string;
  italicVoice: string;
  speed: number;
  paragraphPause: number;
}

/** A sentence to speak; `from`/`to` are only meaningful when `highlight` is set. */
export type Segment = Sentence & { highlight: boolean; pauseAfter?: number; label?: string; voice?: string };

const MAX_CACHED = 400;

export class Narrator {
  private generation = 0;
  private audio: HTMLAudioElement | null = null;
  private finishCurrent: (() => void) | null = null;
  private renders = new Map<string, Promise<ArrayBuffer>>();
  private active = false;

  onSegment: (seg: Segment | null) => void = () => {};
  onPlaying: (playing: boolean) => void = () => {};
  onError: (message: string) => void = () => {};

  constructor(private voice: () => Voice) {}

  get playing(): boolean {
    return this.active;
  }

  private render(text: string, voice: string, speed: number): Promise<ArrayBuffer> {
    const key = `${voice}|${speed}|${text}`;
    let p = this.renders.get(key);
    if (!p) {
      p = api.synth(text, voice, speed);
      p.catch(() => this.renders.delete(key));
      this.renders.set(key, p);
      if (this.renders.size > MAX_CACHED) {
        this.renders.delete(this.renders.keys().next().value!);
      }
    }
    return p;
  }

  /** Forget in-memory renders (the disk cache still has them). Call when the lexicon changes. */
  clearRenders() {
    this.renders.clear();
  }

  stop() {
    this.generation++;
    this.audio?.pause();
    this.audio = null;
    this.finishCurrent?.();
    this.finishCurrent = null;
    if (this.active) {
      this.active = false;
      this.onSegment(null);
      this.onPlaying(false);
    }
  }

  async play(segments: Segment[]) {
    this.stop();
    if (!segments.length) return;
    const gen = this.generation;
    const live = () => gen === this.generation;
    const v = this.voice();
    const request = (i: number) => {
      const s = segments[i];
      return s && this.render(s.speak, s.voice ?? (s.italic ? v.italicVoice : v.voice), v.speed);
    };

    this.active = true;
    this.onPlaying(true);
    try {
      for (let i = 0; i < segments.length; i++) {
        const wav = request(i);
        request(i + 1);
        request(i + 2);
        const buf = await wav;
        if (!live()) return;
        const seg = segments[i];
        this.onSegment(seg);
        await this.playWav(buf);
        if (!live()) return;
        const pause = seg.pauseAfter ?? (seg.paraEnd ? v.paragraphPause : 0);
        if (pause > 0) await new Promise((r) => setTimeout(r, pause * 1000));
        if (!live()) return;
      }
    } catch (e) {
      if (live()) this.onError(String(e));
    }
    if (live()) {
      this.audio = null;
      this.active = false;
      this.onSegment(null);
      this.onPlaying(false);
    }
  }

  private playWav(buf: ArrayBuffer): Promise<void> {
    const url = URL.createObjectURL(new Blob([buf], { type: "audio/wav" }));
    const audio = new Audio(url);
    this.audio = audio;
    return new Promise<void>((resolve) => {
      const done = () => {
        URL.revokeObjectURL(url);
        this.finishCurrent = null;
        resolve();
      };
      this.finishCurrent = done;
      audio.onended = done;
      audio.onerror = done;
      audio.play().catch(done);
    });
  }
}
