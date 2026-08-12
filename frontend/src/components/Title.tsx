/**
 * The heading, in the wordmark's own shape.
 *
 * `Doppelganger` is set in an italic serif with the second half in a lighter tone,
 * and a deployment that named itself gets the same treatment rather than plain
 * text: the title is the thing an operator reads to know which of four tabs they
 * are looking at, and it should look like it belongs to the same page.
 *
 * The tones follow the wordmark: the first word in the strong one, everything after
 * it in the lighter one. Not a decision about which word matters more -- it is what
 * `Doppel` + `ganger` already does, and a title of one word simply gets one tone.
 */

/** The strong tone and the light one, as `Wordmark` uses them. */
const STRONG = 'text-slate-900 dark:text-white'
const LIGHT = 'text-sky-700 dark:text-sky-200'

/** The face, weight and size shared with the wordmark. */
export const TITLE_CLASS = 'font-serif text-lg font-medium italic select-none'

/**
 * A title split into the pieces that get a tone each, with every character kept.
 *
 * Three ways a title says "word boundary", all of them seen in a deployment name:
 * a space (`Billing API`), an underscore (`billing_api`), and a case change
 * (`billingApi`). Hyphens too, because `billing-api` is the same name written the
 * way a hostname is.
 *
 * Separators are kept as they were written and travel with the word that follows
 * them, so `billing_api` reads as `billing_api` with `_api` in the light tone
 * rather than turning into `billing api`. The title is what the operator typed;
 * this only colours it.
 *
 * Exported for its own tests: the interesting cases are all in here, and asserting
 * them through a rendered component would be asserting them through two things.
 */
export function titleSegments(title: string): string[] {
  const segments: string[] = []
  let current = ''

  for (const [index, character] of [...title].entries()) {
    const isSeparator = /[\s_-]/.test(character);
    const previous = [...title][index - 1];
    // A case change is a boundary only between two letters: `API2` is one word,
    // and so is the `A` after a space.
    const camel =
      previous !== undefined &&
      /[a-z0-9]/.test(previous) &&
      /[A-Z]/.test(character) &&
      current !== '';

    if (isSeparator) {
      // The separator starts the next segment rather than ending this one.
      if (current !== '' && !/^[\s_-]+$/.test(current)) {
        segments.push(current)
        current = ''
      }
    } else if (camel) {
      segments.push(current)
      current = ''
    }
    current += character
  }
  if (current !== '') {
    segments.push(current)
  }
  return segments
}

/**
 * The title, or the wordmark when there is no title.
 *
 * `titleIsDefault` decides, and it comes from the server rather than from
 * comparing the string to `Doppel`: a deployment that deliberately named itself
 * `Doppel` is a deployment with a title, and it gets its own name drawn the same
 * way as any other.
 */
export function Title({ title }: { title: string }) {
  const segments = titleSegments(title)
  return (
    <span className={TITLE_CLASS}>
      {segments.map((segment, index) => (
        <span
          // The index is the identity here: two identical words in one title are
          // two segments, and neither is more itself than the other.
          key={`${index}-${segment}`}
          className={index === 0 ? STRONG : LIGHT}
        >
          {segment}
        </span>
      ))}
    </span>
  )
}
