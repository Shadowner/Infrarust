<script setup lang="ts">
const props = defineProps<{
  motd: unknown;
}>();

const colorMap: Record<string, string> = {
  '0': '#000000', '1': '#0000AA', '2': '#00AA00', '3': '#00AAAA',
  '4': '#AA0000', '5': '#AA00AA', '6': '#FFAA00', '7': '#AAAAAA',
  '8': '#555555', '9': '#5555FF', 'a': '#55FF55', 'b': '#55FFFF',
  'c': '#FF5555', 'd': '#FF55FF', 'e': '#FFFF55', 'f': '#FFFFFF',
};

interface MotdSpan {
  text: string;
  color?: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strikethrough?: boolean;
}

function parseMotdString(text: string): MotdSpan[] {
  const spans: MotdSpan[] = [];
  let current: MotdSpan = { text: '' };

  for (let i = 0; i < text.length; i++) {
    if (text[i] === '§' && i + 1 < text.length) {
      if (current.text) spans.push({ ...current });
      const code = text[i + 1]!.toLowerCase();
      i++;

      if (colorMap[code]) {
        current = { text: '', color: colorMap[code] };
      } else if (code === 'l') {
        current = { ...current, text: '', bold: true };
      } else if (code === 'o') {
        current = { ...current, text: '', italic: true };
      } else if (code === 'n') {
        current = { ...current, text: '', underline: true };
      } else if (code === 'm') {
        current = { ...current, text: '', strikethrough: true };
      } else if (code === 'r') {
        current = { text: '' };
      } else {
        current = { ...current, text: '' };
      }
    } else {
      current.text += text[i];
    }
  }
  if (current.text) spans.push(current);
  return spans;
}

const chatColorMap: Record<string, string> = {
  'black': '#000000', 'dark_blue': '#0000AA', 'dark_green': '#00AA00',
  'dark_aqua': '#00AAAA', 'dark_red': '#AA0000', 'dark_purple': '#AA00AA',
  'gold': '#FFAA00', 'gray': '#AAAAAA', 'dark_gray': '#555555',
  'blue': '#5555FF', 'green': '#55FF55', 'aqua': '#55FFFF',
  'red': '#FF5555', 'light_purple': '#FF55FF', 'yellow': '#FFFF55',
  'white': '#FFFFFF',
};

function parseJsonComponent(comp: unknown): MotdSpan[] {
  if (typeof comp === 'string') return parseMotdString(comp);

  if (typeof comp !== 'object' || comp === null) return [];
  const obj = comp as Record<string, unknown>;

  const spans: MotdSpan[] = [];
  const text = (obj.text as string) ?? '';

  if (text) {
    spans.push({
      text,
      color: chatColorMap[obj.color as string] ?? (typeof obj.color === 'string' && obj.color.startsWith('#') ? obj.color : undefined),
      bold: obj.bold as boolean | undefined,
      italic: obj.italic as boolean | undefined,
      underline: obj.underlined as boolean | undefined,
      strikethrough: obj.strikethrough as boolean | undefined,
    });
  }

  if (Array.isArray(obj.extra)) {
    for (const child of obj.extra) {
      spans.push(...parseJsonComponent(child));
    }
  }

  return spans;
}

const spans = computed(() => {
  const motd = props.motd;
  if (typeof motd === 'string') return parseMotdString(motd);
  if (Array.isArray(motd)) return motd.flatMap(parseJsonComponent);
  if (typeof motd === 'object' && motd !== null) return parseJsonComponent(motd);
  return [];
});
</script>

<template>
  <span class="font-mono text-sm leading-relaxed">
    <span
      v-for="(span, i) in spans"
      :key="i"
      :style="{
        color: span.color,
        fontWeight: span.bold ? '700' : undefined,
        fontStyle: span.italic ? 'italic' : undefined,
        textDecoration: [
          span.underline ? 'underline' : '',
          span.strikethrough ? 'line-through' : '',
        ].filter(Boolean).join(' ') || undefined,
      }"
    >{{ span.text }}</span>
  </span>
</template>
