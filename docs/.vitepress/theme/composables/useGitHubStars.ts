import { ref, onMounted } from 'vue'

const cached = ref<number | null>(null)

export function useGitHubStars(repo: string) {
  const stars = ref<number | null>(cached.value)

  onMounted(async () => {
    if (cached.value !== null) {
      stars.value = cached.value
      return
    }
    try {
      const res = await fetch(`https://api.github.com/repos/${repo}`)
      if (res.ok) {
        const data = await res.json()
        cached.value = data.stargazers_count
        stars.value = data.stargazers_count
      }
    } catch {
    }
  })

  return stars
}
