const buttons=penpotUtils.findShapes(s=>s.type==='board'&&s.name.startsWith('Button / '),penpot.root);
for(const b of buttons){b.flex.wrap='nowrap';b.flex.alignContent='center';b.flex.horizontalPadding=8;b.flex.verticalPadding=11;}
for(const b of penpotUtils.findShapes(s=>s.type==='board'&&s.name==='Terminal toolbar / Disabled',penpot.root)){b.flex.wrap='nowrap';b.flex.alignContent='center';b.flex.verticalPadding=9;}
await penpot.waitForLayoutUpdate();
return {buttons:buttons.length};
