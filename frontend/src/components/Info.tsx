import { parameterUrl } from '../services/docs'

/**
 * An (i) beside a field, linking to what the documentation says about it.
 *
 * The field's own hint is one line and says what to type; this is where the rest of
 * it lives -- the bounds, the default, an example in place, and why the setting
 * exists. Putting all of that under every control would make the form unreadable,
 * and leaving it out entirely is how an operator ends up guessing.
 *
 * A link, not a tooltip: it goes to a page that can be read, searched and linked to
 * in a message to a colleague. It opens in a new tab, because leaving the form would
 * throw away whatever is half-typed in it.
 *
 * The href carries the running version and a path-derived anchor. Both are checked
 * by `scripts/check_docs_links.py`, since a wrong fragment is silent in a browser.
 *
 * Small, and centred on the label's line: it is a footnote marker, not a control, and
 * anything bigger reads as something to press instead of something to consult. The
 * rows that hold it use `items-center` with no margin on the label, which is what
 * makes "centred" mean centred on the words rather than on the space under them.
 *
 * Its name deliberately does not repeat the field's. The link sits beside the label,
 * so the field is already said; naming it "what Timeout (seconds) does" instead put
 * that sentence into every locator that looks for a field by its label, in tests and
 * in a screen reader's own field list. The `title` still names the field, for a mouse.
 */
export function Info({ path, label }: { path: string; label: string }) {
  return (
    <a
      href={parameterUrl(path)}
      target="_blank"
      rel="noreferrer"
      aria-label="What this field does"
      title={`${label}: what it does, in the documentation`}
      className="inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full border border-slate-300 align-middle text-[0.5625rem] leading-none text-slate-500 hover:border-teal-500 hover:text-teal-700 dark:border-slate-600 dark:text-slate-400 dark:hover:border-teal-400 dark:hover:text-teal-300"
    >
      i
    </a>
  )
}
