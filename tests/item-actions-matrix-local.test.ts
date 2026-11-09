// The matrix's places on this computer: folders, pseudo places and the Trash.
// Shared setup and rows are in tests/support/item-actions-matrix.ts.
import { beforeAll, beforeEach, expect, test } from "vitest";
import { closeOverlays } from "./app";
import { LOCAL_ROWS, row, setupMatrix } from "./support/item-actions-matrix";

beforeAll(setupMatrix);
beforeEach(closeOverlays);

for (const [name, [go, wanted, expected]] of Object.entries(LOCAL_ROWS)) {
  test(`${name}: every surface`, async () => {
    expect(await row(go, wanted)).toEqual(expected);
  });
}
