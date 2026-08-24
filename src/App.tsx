function App() {
  return (
    <main className="min-h-screen bg-neutral-950 text-neutral-100">
      <div className="mx-auto max-w-5xl px-6 py-8">
        <header className="mb-8">
          <h1 className="text-2xl font-semibold tracking-tight">
            Valorant Lightweight Tracker
          </h1>
          <p className="mt-1 text-sm text-neutral-400">
            In-match player table for your current game.
          </p>
        </header>

        <section
          aria-label="Player table"
          className="rounded-lg border border-neutral-800 bg-neutral-900/50"
        >
          <div className="border-b border-neutral-800 px-4 py-3 text-sm font-medium text-neutral-300">
            Players
          </div>
          <div className="flex h-64 items-center justify-center px-4 text-sm text-neutral-500">
            No match detected. Player table will appear here.
          </div>
        </section>
      </div>
    </main>
  );
}

export default App;
