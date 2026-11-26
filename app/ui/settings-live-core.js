// The models this PC can run for live subtitles. Choosing one is an ordinary settings edit, saved with Save.
export const LIVE_MODELS = [
  { id: 'main', name: "This PC's full model", note: 'Translates and explains words; uses the most video memory.' },
  { id: 'hy_mt2', name: 'HY-MT2 7B', note: 'Translation only · the most careful of the small ones.', path: 'continuous.translation.hy_mt2_path', file: 'Hy-MT2-7B-Q4_K_M.gguf' },
  { id: 'milmmt', name: 'MiLMMT 4B', note: 'Translation only · quick, less video memory.', path: 'continuous.translation.milmmt_path', file: 'MiLMMT-46-4B-v1.0.Q6_K.gguf' },
  { id: 'hy_mt2_small', name: 'HY-MT2 1.8B', note: 'Translation only · little video memory, more mistakes with game terms.', path: 'continuous.translation.hy_mt2_small_path', file: 'Hy-MT2-1.8B-Q4_K_M.gguf' },
  { id: 'lfm', name: 'LFM2 KoEn 1.2B', note: 'Translation only · the smallest, with the most mistakes.', path: 'continuous.translation.lfm_path', file: 'lfm2-1.2b-koen-mt-v8-rl-10k-merged-Q5_K_M.gguf' },
];

/**
 * The edits for choosing a live model on this PC: the choice itself and, for a translation-only model whose
 * file is not set yet, the file Maru found for it (`files`: `[{model, path}]`). A path you typed is kept.
 */
export function modelEdits(id, values, files) {
  const model = LIVE_MODELS.find((m) => m.id === id);
  if (!model) return {};
  const edits = { 'tasks.live.connection': 'local', 'tasks.live.model': id === 'main' ? '' : id };
  if (model.path && !values[model.path]) {
    const path = files?.find((f) => f.model === id)?.path;
    if (path) edits[model.path] = path;
  }
  return edits;
}
