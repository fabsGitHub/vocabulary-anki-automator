/**
 * The live demo's own text, in Learn's two UI languages. It is kept out of
 * the app's message files (`../i18n`) so that no other build carries it;
 * the demo's bar reads it as its local messages, and the entry point shows
 * `preparing` before the app mounts.
 */

export const demoMessages = {
  en: {
    preparing: 'Preparing the demo…',
    label: 'Live demo',
    summary:
      'Ten days of made-up practice are already done. What you do here stays in this browser.',
    summaryMemory:
      'Ten days of made-up practice are already done. This browser does not let the demo keep anything, so a reload starts it over.',
    date: 'Demo date: {date}',
    nextDay: 'Next day',
    nextDayHint: 'Moves the demo’s clock forward by one day, so you can see which cards come back.',
    startOver: 'Start over',
    startOverHint: 'Starts the demo over with its original history and today’s date.',
    failed: 'That did not work: {error}',
    about: 'About this demo',
    aboutApps:
      'Vocabulary Learn is one of two desktop apps on a shared Rust core. Vocabulary Automator turns any word you select on your computer into a flashcard and sends it to Anki; Learn schedules those cards with spaced repetition (FSRS) and adds a daily goal, exercises, lessons, XP and streaks.',
    aboutTech:
      'This browser version runs a TypeScript port of the core over IndexedDB instead of Rust over SQLite, held to the Rust code by golden test vectors. The demo’s history was played through the app’s own commands, and nothing you do here leaves your browser.',
    projectLink: 'Project page',
    promptIntro:
      'Cards that are due for review always come first. Then come the words captured in the Automator, oldest first, and then the HSK word list in order.',
  },
  de: {
    preparing: 'Die Demo wird vorbereitet…',
    label: 'Live-Demo',
    summary:
      'Zehn Tage erfundene Übung sind schon erledigt. Was du hier tust, bleibt in diesem Browser.',
    summaryMemory:
      'Zehn Tage erfundene Übung sind schon erledigt. Dieser Browser lässt die Demo nichts speichern, deshalb beginnt sie nach dem Neuladen von vorn.',
    date: 'Demo-Datum: {date}',
    nextDay: 'Nächster Tag',
    nextDayHint:
      'Stellt die Uhr der Demo einen Tag vor, damit du siehst, welche Karten wiederkommen.',
    startOver: 'Neu starten',
    startOverHint:
      'Startet die Demo mit ihrer ursprünglichen Vorgeschichte und dem heutigen Datum neu.',
    failed: 'Das hat nicht geklappt: {error}',
    about: 'Über diese Demo',
    aboutApps:
      'Vocabulary Learn ist eine von zwei Desktop-Apps auf einem gemeinsamen Rust-Kern. Vocabulary Automator macht aus jedem Wort, das du am Computer markierst, eine Karteikarte und schickt sie an Anki; Learn plant diese Karten mit Spaced Repetition (FSRS) und bringt Tagesziel, Übungen, Lektionen, XP und Streaks mit.',
    aboutTech:
      'Diese Browser-Version führt statt Rust über SQLite eine TypeScript-Portierung des Kerns über IndexedDB aus, die Golden-Testvektoren an den Rust-Code binden. Die Vorgeschichte der Demo wurde mit den Befehlen der App selbst abgespielt, und nichts, was du hier tust, verlässt deinen Browser.',
    projectLink: 'Projektseite',
    promptIntro:
      'Fällige Wiederholungen kommen immer zuerst. Danach kommen die im Automator erfassten Wörter, die ältesten zuerst, und dann die HSK-Wortliste der Reihe nach.',
  },
}

export type DemoMessages = typeof demoMessages.en
