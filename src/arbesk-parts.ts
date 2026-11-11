/**
 * An Arbesk scene's parts fetched a few at a time (performance plan,
 * phase 3), with the outcome of fetching them one after another: parts
 * are taken (parsed) one at a time in the scene's order, and each part's
 * verdict is the one a fetch in turn, given what is left of the budget,
 * gives. No DOM, no Tauri: the caller hands the fetch and the parse.
 *
 * The budget is reserved per call in flight: a call may take at most its
 * share of what no other call has reserved and no landed part holds, so
 * what is held and on its way never passes the budget. A part refused as
 * too large for a share smaller than what a fetch in turn would have
 * given is asked again alone, once the calls in flight have landed and
 * the parts after it are let go (the backend keeps them built), with
 * everything left: so the verdict is the one a fetch in turn gives.
 */

/** How many parts are asked for at once. */
export const PARTS_AT_ONCE = 4;

export interface PartWindow {
  /** How many parts the scene has. */
  count: number;
  /** The bytes every part may take together. */
  budget: number;
  /** Part `index`, at most `limit` bytes (refused as "too large" past it). */
  fetch(index: number, limit: number): Promise<ArrayBuffer>;
  /** Part `index` landed: parsed, one at a time in the scene's order. */
  take(index: number, bytes: ArrayBuffer): Promise<void>;
  /** The preview moved on: no more calls start and nothing more is taken. */
  stale(): boolean;
  /** Calls at once, `PARTS_AT_ONCE` when not given. */
  atOnce?: number;
}

/** One call's answer, and the limit it was asked with. */
type Got = { limit: number; bytes?: ArrayBuffer; error?: unknown };

const isTooLarge = (error: unknown) => /too large/i.test(String(error));

/** Fetch and take the parts `window` names; whether any was too large or
 *  the budget ran out before the last, as the one-at-a-time loop said. */
export async function fetchParts(window: PartWindow): Promise<{ tooLarge: boolean }> {
  return new Fetching(window).run();
}

class Fetching {
  private left: number;
  /** The limits of the calls in flight and the bytes of parts landed but not taken. */
  private reserved = 0;
  /** The next part no call has asked for. */
  private next = 0;
  private readonly asked: (Promise<Got> | undefined)[] = [];
  private readonly landed: (Got | undefined)[] = [];
  private readonly flying = new Set<Promise<Got>>();
  /** A part is asked for alone: nothing else starts. */
  private alone = false;
  private tooLarge = false;
  private readonly atOnce: number;

  constructor(private readonly window: PartWindow) {
    this.left = window.budget;
    this.atOnce = Math.max(1, window.atOnce ?? PARTS_AT_ONCE);
  }

  async run(): Promise<{ tooLarge: boolean }> {
    for (let index = 0; index < this.window.count; index++) {
      if (this.window.stale()) break;
      if (this.left <= 0) {
        this.tooLarge = true;
        break;
      }
      const got = await this.answer(index);
      if (this.window.stale()) break;
      await this.take(index, got);
    }
    return { tooLarge: this.tooLarge };
  }

  /** Part `index`'s answer, asked again alone when its share was too small. */
  private async answer(index: number): Promise<Got> {
    this.fill();
    const asked = this.asked[index];
    const got = asked ? await asked : undefined;
    if (got && !(got.error !== undefined && isTooLarge(got.error) && got.limit < this.left)) return got;
    return this.askAlone(index);
  }

  /** Part `index` asked for with everything left, nothing else in flight:
   *  the calls under way land first and the parts after it are let go. */
  private async askAlone(index: number): Promise<Got> {
    this.alone = true;
    try {
      await Promise.all(this.flying);
      this.letGoAfter(index);
      if (this.window.stale()) return { limit: 0, error: "Cancelled." };
      return await this.call(index, this.left);
    } finally {
      this.alone = false;
    }
  }

  private letGoAfter(index: number): void {
    for (let later = index + 1; later < this.window.count; later++) {
      this.reserved -= this.landed[later]?.bytes?.byteLength ?? 0;
      this.landed[later] = undefined;
      this.asked[later] = undefined;
    }
    this.next = index + 1;
  }

  /** Start calls while slots and budget allow, each with its share. */
  private fill(): void {
    while (!this.alone && !this.window.stale() && this.flying.size < this.atOnce && this.next < this.window.count) {
      const sharing = Math.min(this.atOnce - this.flying.size, this.window.count - this.next);
      const share = Math.floor((this.left - this.reserved) / sharing);
      if (share <= 0) return;
      void this.call(this.next++, share);
    }
  }

  private call(index: number, limit: number): Promise<Got> {
    this.reserved += limit;
    const call: Promise<Got> = this.window.fetch(index, limit).then(
      (bytes) => this.land(call, index, { limit, bytes }),
      (error: unknown) => this.land(call, index, { limit, error }),
    );
    this.flying.add(call);
    this.asked[index] = call;
    return call;
  }

  private land(call: Promise<Got>, index: number, got: Got): Got {
    this.flying.delete(call);
    this.reserved += (got.bytes?.byteLength ?? 0) - got.limit;
    this.landed[index] = got;
    this.fill();
    return got;
  }

  /** Part `index` taken in turn: its bytes count against what is left
   *  whether or not they parse, as in the one-at-a-time loop. */
  private async take(index: number, got: Got): Promise<void> {
    this.landed[index] = undefined;
    if (!got.bytes) {
      this.tooLarge ||= isTooLarge(got.error);
      return;
    }
    this.reserved -= got.bytes.byteLength;
    this.left -= got.bytes.byteLength;
    try {
      await this.window.take(index, got.bytes);
    } catch (err) {
      this.tooLarge ||= isTooLarge(err);
    }
  }
}
