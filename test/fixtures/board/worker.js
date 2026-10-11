// A tiny stand-in board for Yap's tests (SELF_HOSTING.md › A board): a page
// with columns ([data-lane]) and cards ([data-id]), which is what Yap anchors
// live pointers to (web/js/cursors.js), plus the one API that page calls.
const BOARD = {
  lanes: [
    { id: 'l0', name: 'Inbox', items: [{ id: 'g0_0', title: 'First card' }, { id: 'g0_1', title: 'Second card' }] },
    { id: 'l1', name: 'Playing', items: [{ id: 'g1_0', title: 'Third card' }] },
  ],
};

const PAGE = `<!doctype html>
<html><head><meta charset="utf-8"><title>Test board</title>
<style>body{display:flex;gap:16px;margin:16px;font:14px sans-serif}ul{width:220px;min-height:300px;margin:0;padding:8px;list-style:none;background:#eef}li{margin:0 0 8px;padding:12px;background:#fff;border:1px solid #99a}</style>
</head><body><script>
fetch('/api/board').then((r) => r.json()).then(({ board }) => {
  for (const lane of board.lanes) {
    const ul = document.createElement('ul');
    ul.dataset.lane = lane.id;
    for (const item of lane.items) {
      const li = document.createElement('li');
      li.dataset.id = item.id;
      li.textContent = item.title;
      ul.append(li);
    }
    document.body.append(ul);
  }
});
</script></body></html>`;

export default {
  fetch(request) {
    const { pathname } = new URL(request.url);
    if (pathname === '/') return new Response(PAGE, { headers: { 'Content-Type': 'text/html; charset=utf-8' } });
    if (pathname === '/api/board') return Response.json({ board: BOARD });
    return new Response('Not found', { status: 404 });
  },
};
