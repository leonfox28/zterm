// Run in the existing Penpot document after 01–05. Safe to repeat.
if (penpot.currentPage.id !== 'c828d3cf-7d4e-8145-8008-98f4fa037d49') {
  throw new Error('Wrong prototype page');
}
for (const [name, path] of [
  ['20 · 新会话已创建', '~'],
  ['24 · 接管部署会话', '~/deploy'],
]) {
  const board = penpotUtils.findShape(s => s.type === 'board' && s.name === name, penpot.root);
  const subtitle = penpotUtils.findShape(s => s.type === 'text' && s.characters.startsWith('MacStudio'), board);
  subtitle.characters = `MacStudio  ·  ${path}`;
  subtitle.name = subtitle.characters;
}
for (const flow of penpot.currentPage.flows.filter(f => /^Flow \d+$/.test(f.name))) flow.remove();
const boards = penpot.root.children.filter(s => s.type === 'board' && /^\d+ ·/.test(s.name));
return {
  pageId: penpot.currentPage.id,
  boards: boards.map(b => ({ id: b.id, name: b.name, width: b.width, height: b.height, preview: b.showInViewMode })),
  textOverflow: boards.flatMap(b => penpotUtils.findShapes(s => s.type === 'text', b)
    .filter(s => !penpotUtils.isContainedIn(s, b)).map(s => ({ board: b.name, shape: s.name }))),
  flows: penpot.currentPage.flows.map(f => ({ name: f.name, start: f.startingBoard.id })),
};
