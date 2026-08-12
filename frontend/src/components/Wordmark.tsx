/**
 * The project's name, set in two tones.
 *
 * Shown only when `admin.title` is absent: a deployment that named itself gets its
 * name in plain text, because the name is the thing the operator needs to read to
 * know which of four tabs they are looking at.
 *
 * Two spans rather than an image: it is selectable, it scales with the heading it
 * sits in, it needs no asset and no `img-src` in the policy, and it says the same
 * thing to a screen reader as it does to a reader.
 *
 * A system serif, not a webfont. The face is the browser's -- Georgia, Times, Noto
 * Serif, whatever the platform has -- which costs nothing to fetch and nothing in
 * the content security policy, at the price of not being one exact typeface
 * everywhere.
 */
export function Wordmark() {
  return (
    <span className="font-serif text-lg font-medium italic">
      <span className="text-slate-900 dark:text-white">Doppel</span>
      <span className="text-sky-700 dark:text-sky-200">ganger</span>
    </span>
  )
}
