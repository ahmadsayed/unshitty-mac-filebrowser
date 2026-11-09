// The matrix's remote places' contents and the Trash's folder. Shared setup
// and rows are in tests/support/item-actions-matrix.ts.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { closeOverlays } from "./app";
import { REMOTE_ROWS, row, setupMatrix } from "./support/item-actions-matrix";

beforeAll(setupMatrix);
beforeEach(closeOverlays);

for (const [name, [go, wanted, expected]] of Object.entries(REMOTE_ROWS)) {
  test(`${name}: every surface`, async () => {
    expect(await row(go, wanted)).toEqual(expected);
  });
}
