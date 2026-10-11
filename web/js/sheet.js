/*
 * One sheet for every question Yap asks, instead of the browser's confirm()
 * and prompt() (which don't look like Yap, look like a site error on phones and
 * block the page). One markup (#confirm-dialog in index.html):
 * a title, a sentence, an optional line of text to fill in, Cancel and the
 * action. Resolves true/false, or with the text (null if cancelled).
 */
const $ = (id) => document.getElementById(id);

let settle = null; // the open question's resolve, if any
let asksText = false;

/**
 * @param {object} q
 * @param {string} q.title     what's being asked ("Delete this message?")
 * @param {string} [q.text]    one or two calm sentences about what happens
 * @param {string} [q.ok]      the action's label ("Delete")
 * @param {string|null} [q.cancel]  null = no Cancel (for "here's a link to copy")
 * @param {boolean} [q.danger] a destructive action: drawn red, and Cancel is the default
 * @param {{ label?: string, value?: string, placeholder?: string, maxlength?: number, readonly?: boolean }} [q.input]
 * @returns {Promise<boolean | string | null>}
 */
export function confirmSheet({ title, text = '', ok = 'OK', cancel = 'Cancel', danger = false, input = null }) {
  const dialog = $('confirm-dialog');
  settle?.(asksText ? null : false); // a new question replaces an unanswered one
  $('confirm-title').textContent = title;
  $('confirm-text').textContent = text;
  $('confirm-text').hidden = !text;
  dialog.dataset.kind = danger ? 'danger' : 'question'; // (Windows 98 draws its message-box icon from this)
  asksText = Boolean(input);
  const okButton = $('confirm-ok');
  okButton.textContent = ok;
  okButton.classList.toggle('danger', danger);
  okButton.classList.toggle('primary', !danger);
  const cancelButton = $('confirm-cancel');
  cancelButton.hidden = cancel === null;
  cancelButton.textContent = cancel ?? '';
  const field = $('confirm-field');
  const box = $('confirm-input');
  field.hidden = !input;
  if (input) {
    $('confirm-label').textContent = input.label ?? '';
    $('confirm-label').hidden = !input.label;
    box.value = input.value ?? '';
    box.placeholder = input.placeholder ?? '';
    box.readOnly = Boolean(input.readonly);
    if (input.maxlength) box.maxLength = input.maxlength;
    else box.removeAttribute('maxlength');
  }
  return new Promise((resolve) => {
    settle = (answer) => {
      settle = null;
      resolve(answer);
    };
    if (!dialog.open) dialog.showModal();
    if (input) {
      box.focus();
      box.select();
    } else (danger && cancel !== null ? cancelButton : okButton).focus();
  });
}

/** Answer the open question (once). */
function answer(yes) {
  const resolve = settle;
  settle = null;
  resolve?.(asksText ? (yes ? $('confirm-input').value : null) : yes);
}

/**
 * Wire the sheet once (app.js calls this at boot). Answers are taken when the
 * button is pressed, not on the dialog's (asynchronous) close event, so a
 * question asked right after another can't receive the old one's answer.
 */
export function bindConfirmSheet() {
  const dialog = $('confirm-dialog');
  $('confirm-form').addEventListener('submit', (e) => {
    e.preventDefault();
    answer(e.submitter?.value === 'ok');
    dialog.close();
  });
  // Enter in the text box is the action (the form's first button is Cancel).
  $('confirm-input').addEventListener('keydown', (e) => {
    if (e.key !== 'Enter' || e.isComposing) return;
    e.preventDefault();
    answer(true);
    dialog.close();
  });
  dialog.addEventListener('cancel', () => answer(false)); // Esc
  // Closed some other way (another sheet took over): that's a no.
  dialog.addEventListener('close', () => !dialog.open && answer(false));
}
