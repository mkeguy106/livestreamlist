// Pure autocorrect decision logic.
//
// Ported from the Qt app (~/livestream.list.qt/src/livestream_list/chat/spellcheck/checker.py)
// with one critical addition: a cursor-position guard that fixes the
// "autocorrect re-fires when editing a word mid-sentence" bug.
//
// The Qt bug: `is_past` (next char is space + alpha) returned true even
// when the user moved the cursor BACK into a previously-corrected word
// to edit it. As soon as a character was deleted, the substring became
// a "new" misspelling (not in `_corrected_words`), and autocorrect fired
// AGAIN, replacing the user's in-flight edit.
//
// The fix: don't autocorrect a word if the caret is currently inside it.
// Implemented as `caretInside` in `shouldAutocorrect`. Tested via the
// regression assert in this file's DEV-assert block.

/**
 * Apostrophe-expansion table: high-confidence corrections for common
 * apostrophe-less contractions. Lowercase keys; values preserve the
 * apostrophe + intended capitalization.
 *
 * Sourced verbatim from
 * `~/livestream.list.qt/src/livestream_list/chat/spellcheck/checker.py`.
 */
export const APOSTROPHE_EXPANSIONS = new Map([
  ['dont', "don't"],
  ['cant', "can't"],
  ['wont', "won't"],
  ['wouldnt', "wouldn't"],
  ['couldnt', "couldn't"],
  ['shouldnt', "shouldn't"],
  ['hasnt', "hasn't"],
  ['havent', "haven't"],
  ['hadnt', "hadn't"],
  ['doesnt', "doesn't"],
  ['didnt', "didn't"],
  ['isnt', "isn't"],
  ['arent', "aren't"],
  ['wasnt', "wasn't"],
  ['werent', "weren't"],
  ['im', "I'm"],
  ['ill', "I'll"],
  ['ive', "I've"],
  ['id', "I'd"],
  ['youre', "you're"],
  ['youve', "you've"],
  ['youll', "you'll"],
  ['youd', "you'd"],
  ['theyre', "they're"],
  ['theyve', "they've"],
  ['theyll', "they'll"],
  ['theyd', "they'd"],
  ['weve', "we've"],
  ['well', "we'll"],
  ['wed', "we'd"],
  ['hes', "he's"],
  ['shes', "she's"],
  ['its', "it's"],
]);

/**
 * Damerau-Levenshtein edit distance between two strings.
 * Variant: edits are insert / delete / substitute / TRANSPOSE adjacent.
 * Used by the confidence rule "top suggestion is within distance ≤ 1".
 *
 * Returns an integer ≥ 0. Case-sensitive.
 */
export function damerauLevenshtein(a, b) {
  if (a === b) return 0;
  if (a.length === 0) return b.length;
  if (b.length === 0) return a.length;
  const al = a.length;
  const bl = b.length;
  // 2D DP table, (al+1) × (bl+1).
  const d = Array.from({ length: al + 1 }, () => new Array(bl + 1).fill(0));
  for (let i = 0; i <= al; i++) d[i][0] = i;
  for (let j = 0; j <= bl; j++) d[0][j] = j;
  for (let i = 1; i <= al; i++) {
    for (let j = 1; j <= bl; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      d[i][j] = Math.min(
        d[i - 1][j] + 1,        // delete
        d[i][j - 1] + 1,        // insert
        d[i - 1][j - 1] + cost, // substitute
      );
      // Damerau transpose: adjacent swap.
      if (
        i > 1 && j > 1 &&
        a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]
      ) {
        d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + cost);
      }
    }
  }
  return d[al][bl];
}

/**
 * Find the misspelled-range (if any) that contains the caret position.
 * Used by the cursor-position guard.
 *
 * @param {Array<{start: number, end: number, word: string}>} ranges
 * @param {number} caret  byte offset
 * @returns the matching range, or null
 */
export function rangeAtCaret(ranges, caret) {
  for (const r of ranges) {
    // Inclusive-on-both-ends: if caret == r.end, treat as "inside" for
    // the +1 tolerance described in the spec (the caret right at the
    // trailing edge of a word the user just typed, before pressing space).
    if (caret >= r.start && caret <= r.end) return r;
  }
  return null;
}

/**
 * The autocorrect decision. Returns the replacement string (e.g. "the"
 * for "teh") if autocorrect should fire, or `null` if it should not.
 *
 * Conditions ALL must hold for autocorrect to fire (Qt rules + the bug fix):
 *
 * 1. caretInside === false             (BUG FIX — caret not inside this word)
 * 2. isPast === true                   (Qt rule 2 — user moved past via space + alpha)
 * 3. !alreadyCorrected.has(lc(word))   (Qt rule 4 — not already corrected this session)
 * 4. !personalDict.has(lc(word))       (also skip user-dict words)
 * 5. Confident correction exists       (Qt rule 3): apostrophe expansion, OR
 *    suggestions.length === 1, OR
 *    damerauLevenshtein(word, suggestions[0]) <= 1
 *
 * @param {object} input
 * @param {string} input.word                    the misspelled word
 * @param {string[]} input.suggestions           top suggestions from hunspell
 * @param {boolean} input.isPast                 true if text after the word is space + alpha
 * @param {boolean} input.caretInside            true if caret is currently within [word.start, word.end]
 * @param {Set<string>} input.alreadyCorrected   lowercased
 * @param {Set<string>} input.personalDict       lowercased
 * @returns {string|null}
 */
export function shouldAutocorrect({
  word,
  suggestions,
  isPast,
  caretInside,
  alreadyCorrected,
  personalDict,
}) {
  if (caretInside) return null;                                  // ← BUG FIX
  if (!isPast) return null;                                      // Qt rule 2
  const lc = word.toLowerCase();
  if (alreadyCorrected.has(lc)) return null;                     // Qt rule 4
  if (personalDict.has(lc)) return null;

  // Qt rule 3 — confidence.
  if (APOSTROPHE_EXPANSIONS.has(lc)) return APOSTROPHE_EXPANSIONS.get(lc);
  if (!suggestions || suggestions.length === 0) return null;
  if (suggestions.length === 1) return suggestions[0];
  if (damerauLevenshtein(word, suggestions[0]) <= 1) return suggestions[0];
  return null;
}

/**
 * `isPast` helper — true iff text[end] is a space AND text[end+1] is
 * an ASCII alpha. Mirrors Qt's `is_past` exactly (no other characters
 * trigger autocorrect — e.g. punctuation or end-of-string don't).
 */
export function isPastWord(text, end) {
  if (end >= text.length) return false;
  if (text[end] !== ' ') return false;
  const next = text[end + 1];
  if (!next) return false;
  return /[a-zA-Z]/.test(next);
}

// ── Module-scope DEV asserts (run once on import in dev) ──────────────────
if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  // damerauLevenshtein
  console.assert(damerauLevenshtein('', '') === 0, 'dl: empty/empty');
  console.assert(damerauLevenshtein('abc', 'abc') === 0, 'dl: equal');
  console.assert(damerauLevenshtein('teh', 'the') === 1, 'dl: teh→the (transpose)');
  console.assert(damerauLevenshtein('cat', 'cats') === 1, 'dl: insert');
  console.assert(damerauLevenshtein('cats', 'cat') === 1, 'dl: delete');
  console.assert(damerauLevenshtein('cat', 'bat') === 1, 'dl: substitute');
  console.assert(damerauLevenshtein('cat', 'dog') === 3, 'dl: 3 subs');

  // isPastWord
  console.assert(isPastWord('teh hello', 3) === true, 'isPast: teh|hello');
  console.assert(isPastWord('teh ', 3) === false, 'isPast: teh + space + EOL → no');
  console.assert(isPastWord('teh!', 3) === false, 'isPast: punct after → no');
  console.assert(isPastWord('teh', 3) === false, 'isPast: end of string → no');

  // rangeAtCaret — cursor-position guard primitive
  const ranges = [{ start: 6, end: 15, word: 'wnoderful' }];
  console.assert(rangeAtCaret(ranges, 5) === null, 'rangeAtCaret: before word');
  console.assert(rangeAtCaret(ranges, 16) === null, 'rangeAtCaret: after word');
  console.assert(rangeAtCaret(ranges, 6) !== null, 'rangeAtCaret: at word.start');
  console.assert(rangeAtCaret(ranges, 10) !== null, 'rangeAtCaret: middle');
  console.assert(rangeAtCaret(ranges, 15) !== null, 'rangeAtCaret: at word.end (+1 tolerance)');

  // shouldAutocorrect — happy path: teh → the
  const empty = new Set();
  console.assert(
    shouldAutocorrect({
      word: 'teh',
      suggestions: ['the', 'eh', 'ten'],
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === 'the',
    'autocorrect: teh→the (DL=1)',
  );

  // shouldAutocorrect — apostrophe expansion (highest priority)
  console.assert(
    shouldAutocorrect({
      word: 'dont',
      suggestions: ['done', 'donut'],
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === "don't",
    'autocorrect: dont→don\'t (apostrophe expansion beats suggestions)',
  );

  // shouldAutocorrect — single suggestion
  console.assert(
    shouldAutocorrect({
      word: 'helo',
      suggestions: ['hello'],  // only one
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === 'hello',
    'autocorrect: single-suggestion path',
  );

  // shouldAutocorrect — multiple suggestions, top NOT within DL=1 → null
  console.assert(
    shouldAutocorrect({
      word: 'xyzq',
      suggestions: ['hello', 'world', 'cat'],  // none close
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === null,
    'autocorrect: low-confidence → no fire',
  );

  // shouldAutocorrect — !isPast → null
  console.assert(
    shouldAutocorrect({
      word: 'teh',
      suggestions: ['the'],
      isPast: false,  // user still typing this word
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === null,
    'autocorrect: !isPast → no fire',
  );

  // shouldAutocorrect — already corrected this session → null
  console.assert(
    shouldAutocorrect({
      word: 'teh',
      suggestions: ['the'],
      isPast: true,
      caretInside: false,
      alreadyCorrected: new Set(['teh']),  // already
      personalDict: empty,
    }) === null,
    'autocorrect: alreadyCorrected → no fire',
  );

  // shouldAutocorrect — in personal dict → null
  console.assert(
    shouldAutocorrect({
      word: 'kappa',
      suggestions: ['kappa'],
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: new Set(['kappa']),
    }) === null,
    'autocorrect: personalDict → no fire',
  );

  // ★ THE BUG REGRESSION ★
  // Scenario: user typed "teh hello", autocorrect fired, replaced "teh"
  // with "the". User clicks back into "the", deletes one char → "te".
  // The substring "te" is now flagged misspelled. is_past is still true
  // (text after "te" is space + "h"). "te" is NOT in alreadyCorrected
  // (only "teh" is). The Qt bug would re-fire autocorrect here.
  // The fix: caretInside === true → null.
  console.assert(
    shouldAutocorrect({
      word: 'te',
      suggestions: ['the', 'tea', 'ted'],  // confident
      isPast: true,           // text after is " hello" — space + alpha
      caretInside: true,      // ← THE FIX
      alreadyCorrected: new Set(['teh']),  // only the original "teh" is recorded
      personalDict: empty,
    }) === null,
    'autocorrect: BUG REGRESSION — caret inside word → no fire',
  );
}
