// Two short rising notes (E5 then A5) made with Web Audio. No audio files, no network.
const NOTES: [number, number][] = [
  [659.25, 0],
  [880, 0.11],
];
const NOTE_LEN = 0.11;
const GAIN = 0.12;

let ctx: AudioContext | null = null;
let unlockBound = false;

function getContext(): AudioContext | null {
  if (ctx) return ctx;
  try {
    const Ctor: typeof AudioContext | undefined =
      window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!Ctor) return null;
    ctx = new Ctor();
  } catch {
    ctx = null;
  }
  return ctx;
}

/** Resume the audio context on the first pointer or key event. Safe to call more than once. */
export function bindAudioUnlock(): void {
  if (unlockBound || typeof window === 'undefined') return;
  unlockBound = true;
  const unlock = () => {
    window.removeEventListener('pointerdown', unlock, true);
    window.removeEventListener('keydown', unlock, true);
    try {
      void getContext()?.resume().catch(() => {});
    } catch {
      // audio unavailable
    }
  };
  window.addEventListener('pointerdown', unlock, true);
  window.addEventListener('keydown', unlock, true);
}

/** Play the chime. Does nothing if audio is unavailable. */
export function playChime(): void {
  try {
    const c = getContext();
    if (!c) return;
    if (c.state === 'suspended') void c.resume().catch(() => {});
    const t0 = c.currentTime + 0.01;
    for (const [freq, offset] of NOTES) {
      const osc = c.createOscillator();
      const gain = c.createGain();
      const start = t0 + offset;
      const end = start + NOTE_LEN;
      osc.type = 'sine';
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0, start);
      gain.gain.linearRampToValueAtTime(GAIN, start + 0.015);
      gain.gain.setValueAtTime(GAIN, end - 0.04);
      gain.gain.linearRampToValueAtTime(0, end);
      osc.connect(gain).connect(c.destination);
      osc.start(start);
      osc.stop(end + 0.01);
    }
  } catch {
    // audio unavailable
  }
}
