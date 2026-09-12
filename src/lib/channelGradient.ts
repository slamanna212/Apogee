export const GRADIENTS: [string, string][] = [
  ['#ff7a5c', '#8b6bff'],
  ['#45e0d8', '#5c7cfa'],
  ['#f76707', '#f06595'],
  ['#12b886', '#339af0'],
  ['#5c7cfa', '#8b6bff'],
  ['#9775fa', '#f06595'],
  ['#fcc419', '#f76707'],
  ['#339af0', '#12b886'],
];

export function hashGradient(seed: string): string {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) | 0;
  const [a, b] = GRADIENTS[Math.abs(hash) % GRADIENTS.length];
  return `linear-gradient(150deg, ${a}, ${b})`;
}
