// Saves of one file (starred.json, pins.json), one write at a time, so the
// last write carries the latest state (#172). Two writes sent close together
// can reach the backend in either order, and the older list would win; here
// a save asked for while a write runs waits for it to end, and the next
// write reads the state only as it starts, so every save asked for meanwhile
// becomes that one write. No DOM: main.ts says what to read, how to write
// and how to report a failure.

/** One file's saves: one write at a time, each with the state as it is when it starts. */
export class LatestSave<T> {
  /** A write is running. */
  private writing = false;
  /** The saves asked for while it runs: the next write ends them all. */
  private queued: { done: Promise<boolean>; end: (saved: boolean) => void } | null = null;

  constructor(
    private readonly read: () => T,
    private readonly write: (state: T) => Promise<void>,
    private readonly failed: (err: unknown) => void,
  ) {}

  /** Save the state. Resolves once a write that read the state after this
   *  call has ended: true when it was written. A write that fails goes to
   *  `failed`, once. */
  save(): Promise<boolean> {
    if (!this.writing) return this.run();
    if (!this.queued) {
      let end!: (saved: boolean) => void;
      const done = new Promise<boolean>((resolve) => (end = resolve));
      this.queued = { done, end };
    }
    return this.queued.done;
  }

  private async run(): Promise<boolean> {
    this.writing = true;
    try {
      await this.write(this.read());
      return true;
    } catch (err) {
      this.failed(err);
      return false;
    } finally {
      this.writing = false;
      this.next();
    }
  }

  /** The saves asked for while a write ran: one more write, for them all. */
  private next(): void {
    const queued = this.queued;
    if (!queued) return;
    this.queued = null;
    void this.run().then(queued.end, () => queued.end(false));
  }
}
