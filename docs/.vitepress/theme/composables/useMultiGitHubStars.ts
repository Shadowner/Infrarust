import { ref, reactive, onMounted } from 'vue'

const cache = reactive<Record<string, number | null>>({})

export function useMultiGitHubStars(repos: string[]) {
  const stars = reactive<Record<string, number | null>>({})

  for (const repo of repos) {
    stars[repo] = cache[repo] ?? null
  }

  onMounted(async () => {
    const toFetch = repos.filter((r) => cache[r] === undefined)
    await Promise.allSettled(
      toFetch.map(async (repo) => {
        try {
          const res = await fetch(`https://api.github.com/repos/${repo}`)
          if (res.ok) {
            const data = await res.json()
            cache[repo] = data.stargazers_count
            stars[repo] = data.stargazers_count
          }
        } catch {}
      }),
    )
  })

  return stars
}

export function formatStars(count: number | null | undefined): string {
  if (count == null) return ''
  if (count >= 1000) return (count / 1000).toFixed(1).replace(/\.0$/, '') + 'k'
  return String(count)
}
