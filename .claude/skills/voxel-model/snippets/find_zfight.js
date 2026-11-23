(() => {
  const V = THREE.Vector3; const faces = [];
  Cube.all.forEach(c => {
    if (!c.mesh) return; c.mesh.updateMatrixWorld(true);
    const o = c.origin, a = [c.from[0] - o[0], c.from[1] - o[1], c.from[2] - o[2]], b = [c.to[0] - o[0], c.to[1] - o[1], c.to[2] - o[2]];
    const W = (x, y, z) => c.mesh.localToWorld(new V(x, y, z));
    const defs = {east: [[b[0],a[1],a[2]],[b[0],b[1],a[2]],[b[0],a[1],b[2]],[1,0,0]], west: [[a[0],a[1],a[2]],[a[0],b[1],a[2]],[a[0],a[1],b[2]],[-1,0,0]], up: [[a[0],b[1],a[2]],[b[0],b[1],a[2]],[a[0],b[1],b[2]],[0,1,0]], down: [[a[0],a[1],a[2]],[b[0],a[1],a[2]],[a[0],a[1],b[2]],[0,-1,0]], south: [[a[0],a[1],b[2]],[b[0],a[1],b[2]],[a[0],b[1],b[2]],[0,0,1]], north: [[a[0],a[1],a[2]],[b[0],a[1],a[2]],[a[0],b[1],a[2]],[0,0,-1]]};
    Object.entries(defs).forEach(([f, [p0, p1, p2, n]]) => {
      const P0 = W(...p0), P1 = W(...p1), P2 = W(...p2);
      faces.push({c, f, P0, e1: P1.clone().sub(P0), e2: P2.clone().sub(P0), N: new V(...n).transformDirection(c.mesh.matrixWorld)});
    });
  });
  const hits = [];
  for (let i = 0; i < faces.length; i++) for (let j = i + 1; j < faces.length; j++) {
    const A = faces[i], B = faces[j]; if (A.c === B.c || A.N.dot(B.N) < 0.999) continue;
    if (Math.abs(A.N.dot(A.P0) - A.N.dot(B.P0)) > 0.002) continue;
    const u = A.e1.clone().normalize(), v = A.e2.clone().normalize(), lu = A.e1.length(), lv = A.e2.length();
    const pts = [B.P0, B.P0.clone().add(B.e1), B.P0.clone().add(B.e2), B.P0.clone().add(B.e1).add(B.e2)].map(p => { const d = p.clone().sub(A.P0); return [d.dot(u), d.dot(v)]; });
    const ou = Math.min(lu, Math.max(...pts.map(p => p[0]))) - Math.max(0, Math.min(...pts.map(p => p[0])));
    const ov = Math.min(lv, Math.max(...pts.map(p => p[1]))) - Math.max(0, Math.min(...pts.map(p => p[1])));
    if (ou > 0.01 && ov > 0.01) hits.push([A.c.name, B.c.name, A.f, +(ou * ov).toFixed(2)]);
  }
  return {count: hits.length, hits};
})()
