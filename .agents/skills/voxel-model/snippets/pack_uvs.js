((texName, pxPerUnit = 2, uvSize = 64) => {
  const tex = Texture.all.find(t => t.name === texName);
  const step = 1 / pxPerUnit; const r = v => Math.max(step, Math.ceil(v / step - 1e-6) * step);
  const dims = (c, f) => { const s = c.size(); if (f === 'north' || f === 'south') return [s[0], s[1]]; if (f === 'east' || f === 'west') return [s[2], s[1]]; return [s[0], s[2]]; };
  const items = []; Cube.all.forEach(c => ['north','south','east','west','up','down'].forEach(f => { const d = dims(c, f); items.push({c, f, w: r(d[0]), h: r(d[1])}); }));
  items.sort((a, b) => b.h - a.h || b.w - a.w); let x = 0, y = 0, rowH = 0;
  items.forEach(it => { if (x + it.w > uvSize) { x = 0; y += rowH; rowH = 0; } it.u = x; it.v = y; x += it.w; rowH = Math.max(rowH, it.h); });
  if (y + rowH > uvSize) return {error: 'overflow', needed: y + rowH};
  Undo.initEdit({elements: Cube.all.slice()});
  Cube.all.forEach(c => c.autouv = 0);
  items.forEach(it => { it.c.faces[it.f].texture = tex.uuid; it.c.faces[it.f].uv = [it.u, it.v, it.u + it.w, it.v + it.h]; });
  Undo.finishEdit('Pack UVs'); Canvas.updateAll();
  return {faces: items.length, usedRows: y + rowH};
})('TEXTURE_NAME')
