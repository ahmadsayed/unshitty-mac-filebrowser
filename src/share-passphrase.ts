// Passphrases exist only in these fields and the submitted command. The Rust
// link module owns the envelope, cryptography and protection detection.
import { openDialog } from "./ui";

function passphraseError(value: string): string {
  if (!value) return "Enter a passphrase.";
  if (new TextEncoder().encode(value).length > 1024) return "Use a passphrase of at most 1,024 bytes.";
  return "";
}

/** Optional protection for new invitations; null from read means invalid. */
export function createShareProtection(): {
  element: HTMLFieldSetElement;
  read(): string | undefined | null;
  clear(): void;
} {
  const element = document.createElement("fieldset");
  element.className = "share-protection";
  element.innerHTML =
    `<label class="check-row"><input id="share-protect" type="checkbox" /> Protect new links with a passphrase</label>` +
    `<div class="share-passphrase-fields" hidden>` +
      `<div class="entry-row"><label for="share-passphrase">Passphrase</label>` +
      `<input id="share-passphrase" type="password" autocomplete="new-password" spellcheck="false" /></div>` +
      `<div class="entry-row"><label for="share-passphrase-confirm">Confirm passphrase</label>` +
      `<input id="share-passphrase-confirm" type="password" autocomplete="new-password" spellcheck="false" /></div>` +
      `<p class="share-protection-hint">Use a strong passphrase and send it separately. Existing links keep their access.</p>` +
      `<div class="field-message error" role="status"></div>` +
    `</div>`;
  const toggle = element.querySelector<HTMLInputElement>("#share-protect")!;
  const fields = element.querySelector<HTMLElement>(".share-passphrase-fields")!;
  const input = element.querySelector<HTMLInputElement>("#share-passphrase")!;
  const confirm = element.querySelector<HTMLInputElement>("#share-passphrase-confirm")!;
  const message = element.querySelector<HTMLElement>(".field-message")!;
  const clear = () => {
    input.value = "";
    confirm.value = "";
    message.textContent = "";
  };
  toggle.addEventListener("change", () => {
    fields.hidden = !toggle.checked;
    clear();
    if (toggle.checked) input.focus();
  });
  fields.addEventListener("input", () => { message.textContent = ""; });
  return {
    element,
    clear,
    read() {
      if (!toggle.checked) return undefined;
      const error = passphraseError(input.value);
      message.textContent = error || (input.value !== confirm.value ? "Passphrases do not match." : "");
      if (!message.textContent) return input.value;
      (error ? input : confirm).focus();
      return null;
    },
  };
}

/** Prompt once before claiming a protected invitation. Failures stay inline;
 * Cancel returns null, and an in-flight submission cannot be duplicated. */
export function promptSharePassphrase<T>(submit: (passphrase: string) => Promise<T>): Promise<T | null> {
  return new Promise((resolve) => {
    const extra = document.createElement("div");
    extra.innerHTML =
      `<div class="entry-row"><label for="share-unlock-passphrase">Passphrase</label>` +
      `<input id="share-unlock-passphrase" type="password" autocomplete="off" spellcheck="false" /></div>` +
      `<div class="field-message" role="status"></div>`;
    const input = extra.querySelector<HTMLInputElement>("input")!;
    const message = extra.querySelector<HTMLElement>(".field-message")!;
    let busy = false;
    let dialog: ReturnType<typeof openDialog>;
    const update = () => dialog.setResponseEnabled("unlock", !busy && !!input.value);
    const unlock = async () => {
      const error = passphraseError(input.value);
      message.classList.toggle("error", !!error);
      message.textContent = error;
      if (error) return;
      busy = true;
      input.disabled = true;
      dialog.setResponseEnabled("cancel", false);
      update();
      message.textContent = "Unlocking…";
      try {
        const result = await submit(input.value);
        input.value = "";
        dialog.close();
        resolve(result);
      } catch (err) {
        message.classList.add("error");
        message.textContent = err instanceof Error ? err.message : String(err);
      } finally {
        busy = false;
        input.disabled = false;
        dialog.setResponseEnabled("cancel", true);
        update();
        if (extra.isConnected) {
          input.focus();
          input.select();
        }
      }
    };
    input.addEventListener("input", () => {
      message.textContent = "";
      update();
    });
    dialog = openDialog({
      heading: "Enter Passphrase",
      body: "This shared folder is protected by a passphrase. Ask the person who shared it for the passphrase.",
      extra,
      responses: [
        { id: "cancel", label: "Cancel" },
        { id: "unlock", label: "Unlock", style: "suggested" },
      ],
      defaultResponse: "unlock",
      closeResponse: "cancel",
      onResponse: (id) => {
        if (busy) return false;
        if (id === "unlock") {
          void unlock();
          return false;
        }
        input.value = "";
        resolve(null);
      },
    });
    update();
  });
}
