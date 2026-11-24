const { chromium } = require('playwright');
(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const url = 'file://' + __dirname + '/mockup.html';
  for (const scene of ['lcd', 'led', 'map', 'alarm']) {
    await page.goto(url + '#' + scene);
    await page.reload();
    await page.waitForSelector('body[data-ready="1"]');
    const names = { lcd: 'app-lcd', led: 'app-led', map: 'room-map', alarm: 'alarm' };
    await page.screenshot({ path: __dirname + '/../../docs/screenshots/' + names[scene] + '.png' });
    console.log('rendered', scene);
  }
  await browser.close();
})();
