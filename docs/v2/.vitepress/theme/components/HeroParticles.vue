<script setup>
import { ref, onMounted, onUnmounted } from 'vue'

const canvasRef = ref(null)
let animationId = null
let resizeObserver = null

onMounted(() => {
  const canvas = canvasRef.value
  if (!canvas) return

  const ctx = canvas.getContext('2d')
  if (!ctx) return

  const PARTICLE_COUNT = 80
  const PARTICLE_SIZE = 3
  const REPEL_RADIUS = 120
  const REPEL_STRENGTH = 2

  let mouse = { x: -9999, y: -9999 }
  let particles = []

  function resize() {
    const rect = canvas.parentElement.getBoundingClientRect()
    canvas.width = rect.width
    canvas.height = rect.height
  }

  function createParticle() {
    const speed = 0.3 + Math.random() * 0.5
    const angle = Math.random() * Math.PI * 2
    return {
      x: Math.random() * canvas.width,
      y: Math.random() * canvas.height,
      vx: Math.cos(angle) * speed,
      vy: Math.sin(angle) * speed,
      opacity: 0.25 + Math.random() * 0.2,
    }
  }

  function init() {
    resize()
    particles = []
    for (let i = 0; i < PARTICLE_COUNT; i++) {
      particles.push(createParticle())
    }
  }

  function animate() {
    ctx.clearRect(0, 0, canvas.width, canvas.height)

    for (let i = 0; i < particles.length; i++) {
      const p = particles[i]

      // Mouse repulsion
      const dx = p.x - mouse.x
      const dy = p.y - mouse.y
      const dist = Math.sqrt(dx * dx + dy * dy)
      if (dist < REPEL_RADIUS && dist > 0) {
        const force = (1 - dist / REPEL_RADIUS) * REPEL_STRENGTH
        p.vx += (dx / dist) * force
        p.vy += (dy / dist) * force
      }

      // Dampen velocity back toward base speed
      p.vx *= 0.98
      p.vy *= 0.98

      // Ensure minimum drift
      const speed = Math.sqrt(p.vx * p.vx + p.vy * p.vy)
      if (speed < 0.3) {
        const angle = Math.atan2(p.vy, p.vx)
        p.vx = Math.cos(angle) * 0.3
        p.vy = Math.sin(angle) * 0.3
      }

      p.x += p.vx
      p.y += p.vy

      // Wrap around edges
      if (p.x < -PARTICLE_SIZE) p.x = canvas.width + PARTICLE_SIZE
      if (p.x > canvas.width + PARTICLE_SIZE) p.x = -PARTICLE_SIZE
      if (p.y < -PARTICLE_SIZE) p.y = canvas.height + PARTICLE_SIZE
      if (p.y > canvas.height + PARTICLE_SIZE) p.y = -PARTICLE_SIZE

      // Draw square particle
      ctx.fillStyle = `rgba(232, 131, 42, ${p.opacity})`
      ctx.fillRect(p.x - PARTICLE_SIZE / 2, p.y - PARTICLE_SIZE / 2, PARTICLE_SIZE, PARTICLE_SIZE)
    }

    animationId = requestAnimationFrame(animate)
  }

  canvas.addEventListener('mousemove', (e) => {
    const rect = canvas.getBoundingClientRect()
    mouse.x = e.clientX - rect.left
    mouse.y = e.clientY - rect.top
  })

  canvas.addEventListener('mouseleave', () => {
    mouse.x = -9999
    mouse.y = -9999
  })

  resizeObserver = new ResizeObserver(() => {
    resize()
  })
  resizeObserver.observe(canvas.parentElement)

  init()
  animate()
})

onUnmounted(() => {
  if (animationId != null) {
    cancelAnimationFrame(animationId)
  }
  if (resizeObserver) {
    resizeObserver.disconnect()
  }
})
</script>

<template>
  <canvas ref="canvasRef" class="hero-particles" />
</template>

<style scoped>
.hero-particles {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  pointer-events: auto;
}
</style>
