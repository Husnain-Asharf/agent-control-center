type Write = (bytes: Uint8Array) => void;

/**
 * Joins the attach scrollback with live output so every byte shows exactly once.
 * Chunks carry their stream offset. Before `attach` they queue. After it, each
 * chunk is trimmed against `end`, the stream position written so far.
 */
export class OutputSplicer {
  private end: number | null = null;
  private queue: { bytes: Uint8Array; offset: number }[] = [];

  push(bytes: Uint8Array, offset: number, write: Write): void {
    if (this.end === null) {
      this.queue.push({ bytes, offset });
      return;
    }
    const stop = offset + bytes.length;
    if (stop <= this.end) return;
    // offset > end is a gap: write whole and jump forward.
    write(offset < this.end ? bytes.subarray(this.end - offset) : bytes);
    this.end = stop;
  }

  /** Call after the scrollback is written. `end` comes from the attach reply. */
  attach(end: number, write: Write): void {
    this.end = end;
    const queued = this.queue;
    this.queue = [];
    for (const q of queued) this.push(q.bytes, q.offset, write);
  }
}
