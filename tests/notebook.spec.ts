import {test, expect} from '@playwright/test';
import {execFileSync, spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createInterface} from 'node:readline';
import {
  copyFile,
  mkdir,
  truncate,
  mkdtemp,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import {constants} from 'node:fs';
import {tmpdir} from 'node:os';
import {basename, dirname, join} from 'node:path';
async function fixtureNotebook(
  agents = false,
  datedHistory?: {markdown: string; scores: object[]},
  existingFolder?: string,
  event?: (message: any) => void,
) {
  const folder = existingFolder ?? (await mkdtemp(join(tmpdir(), 'sumi-ui-')));
  if (datedHistory) {
    await mkdir(join(folder, 'scroll'), {recursive: true});
    await writeFile(join(folder, 'scroll/deciding.md'), datedHistory.markdown);
    await writeFile(
      join(folder, 'predictions.json'),
      JSON.stringify(datedHistory.scores),
    );
  }
  const core = spawn(
    'src-tauri/target/debug/sumi',
    [
      '--fixture-rpc',
      '--notebook',
      folder,
      ...(agents ? ['--fixture-agents'] : []),
      ...(datedHistory ? ['--fixture-predictions'] : []),
    ],
    {stdio: ['pipe', 'pipe', 'inherit']},
  );
  const pending: ((s: string) => void)[] = [];
  createInterface({input: core.stdout}).on('line', s => {
    const m = JSON.parse(s);
    if (m.event) {
      event?.(m);
      return;
    }
    pending.shift()?.(s);
  });
  let stopped = '';
  function stop(reason: string) {
    stopped = reason;
    for (const finish of pending.splice(0))
      finish(JSON.stringify({error: reason}));
  }
  core.once('error', e => stop(e.message));
  core.stdin.on('error', e => stop(e.message));
  core.once('close', (code, signal) =>
    stop('Notebook fixture closed (' + (signal || code) + ').'),
  );
  function rpc(op: string, args = {}) {
    return new Promise<any>((resolve, reject) => {
      if (stopped) return reject(Error(stopped));
      pending.push(s => {
        const result = JSON.parse(s);
        if (result.error) reject(Error(result.error));
        else resolve(result.ok);
      });
      core.stdin.write(
        JSON.stringify({operation: op, args, stream: !!event}) + '\n',
      );
    });
  }
  return {core, rpc, folder};
}

// Exercise automatic discovery with a predictable installed model on the test Mac.
function withTestModel(rpc: (op: string, args?: object) => Promise<any>) {
  return async (op: string, args: object) => {
    const result = await rpc(op, args);
    return op === 'models'
      ? result.filter((m: {name: string}) => m.name === 'llama3.2:3b')
      : result;
  };
}

test('brushstroke, real local prediction, correction, history and persisted sheets', async ({
  page,
}, info) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    await page.exposeFunction('sumiTestIPC', withTestModel(rpc));
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page.getByRole('button', {name: 'Think on this Mac instead'}).click();
    await page.getByRole('button', {name: 'Continue', exact: true}).click();
    await expect(page.getByText('A mark of your own.')).toBeVisible();
    const box = await page
      .getByRole('img', {name: /Draw your personal/})
      .boundingBox();
    if (!box) throw Error('Missing drawing surface');
    await page.mouse.move(box.x + 35, box.y + 70);
    await page.mouse.down();
    await page.mouse.move(box.x + 180, box.y + 90, {steps: 16});
    await page.mouse.up();
    await page.getByRole('button', {name: 'Keep my mark'}).click();
    await page.getByRole('button', {name: 'Teach a correction'}).click();
    await expect(page.getByLabel('Your correction')).toBeVisible();
    await expect(
      page.getByRole('button', {name: /export|chat history/i}),
    ).toHaveCount(0);
    await page
      .getByLabel('Your correction')
      .fill(
        'For a print proof, I asked for uncoated paper so I could mark it in pencil.',
      );
    await page.getByRole('button', {name: 'Read my correction'}).click();
    const learned = page.getByLabel('What Sumi learned');
    await expect(learned).toHaveValue(/paper|pencil/i, {timeout: 90000});
    expect((await rpc('state')).entries).toHaveLength(0);
    const rule =
      'For pencil proofing, I always choose uncoated paper over glossy paper.';
    await learned.fill(rule);
    await page.getByRole('button', {name: 'Keep this sentence'}).click();
    await expect(page.getByText(rule, {exact: true})).toBeVisible();
    await page.reload();
    await page
      .getByLabel('Your next decision')
      .fill('For the pencil proof, should I order glossy or uncoated paper?');
    await page.getByRole('button', {name: 'Make a guess'}).click();
    await expect(page.getByLabel('Your decision')).toBeVisible({
      timeout: 90000,
    });
    const hidden = await rpc('state');
    expect(hidden.sealed.guess).toBeUndefined();
    expect(hidden.sealed.ruleIds).toContain(hidden.firstCorrection.id);
    expect(hidden.scores).toHaveLength(0);
    await page.reload();
    await page.getByLabel('Your decision').fill('Order uncoated paper.');
    await page.getByRole('button', {name: 'Open the seal'}).click();
    await expect(page.getByText('The sentence Sumi read')).toBeVisible();
    expect((await rpc('state')).sealed.guess).toMatch(/uncoated/i);
    await page.screenshot({path: info.outputPath('correction-reveal.png')});
    await page.getByRole('button', {name: 'It matched'}).click();
    await page.getByRole('button', {name: 'Open the Desk'}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    const completed = await rpc('state');
    expect(completed.entries).toHaveLength(1);
    expect(completed.firstCorrection).toBeNull();
    await page.keyboard.press('Meta+,');
    await page.getByRole('button', {name: 'Start with a correction'}).click();
    await expect(page.getByLabel('Your correction')).toHaveValue('');
    await page.getByRole('button', {name: 'Return to my notebook'}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    expect(completed.sources).toHaveLength(0);
    expect(completed.requestedExport).toBeNull();
    await rpc('newSheet');
    await page.reload();
    await page
      .getByLabel('Desk sheet')
      .fill('Should I keep the meeting brief or make it detailed');
    await page.getByRole('button', {name: 'Seal a guess'}).click();
    await page.getByRole('button', {name: 'Open sealed guess'}).click();
    await page.getByLabel('Your decision').fill('Keep it brief.');
    await page.getByRole('button', {name: 'Break the seal'}).click();
    await expect(page.getByText('You decided · Keep it brief.')).toBeVisible();
    await page.getByRole('button', {name: 'It missed'}).click();
    await page.keyboard.press('Meta+3');
    await expect(
      page.getByText('1 of 2 scored guesses matched.'),
    ).toBeVisible();
    await page.keyboard.press('Meta+2');
    await page.getByRole('button', {name: 'Write an entry'}).click();
    await page
      .getByLabel('Your sentence')
      .fill('I choose short meetings when the agenda is clear.');
    await page.getByRole('button', {name: 'Keep in the Scroll'}).click();
    await page
      .getByRole('button', {
        name: 'I choose short meetings when the agenda is clear.',
      })
      .click();
    await page.getByRole('button', {name: 'Edit sentence'}).click();
    await page
      .getByLabel('Correct this entry')
      .fill('I prefer an agenda before accepting a meeting.');
    await page.getByRole('button', {name: 'Save correction'}).click();
    await expect(
      page.getByRole('button', {
        name: 'I prefer an agenda before accepting a meeting.',
      }),
    ).toBeVisible();
    const range = page.getByLabel('Scroll history');
    await range.focus();
    await range.press('End');
    await range.press('ArrowLeft');
    await range.press('ArrowLeft');
    await expect(
      page.getByRole('button', {
        name: 'I choose short meetings when the agenda is clear.',
      }),
    ).toBeVisible();
    await range.press('End');
    await range.blur();
    await expect(
      page.getByRole('button', {
        name: 'I prefer an agenda before accepting a meeting.',
      }),
    ).toBeVisible();
    await page.screenshot({
      path: info.outputPath('scroll.png'),
      animations: 'disabled',
    });
    await page.getByRole('button', {name: 'Places'}).click();
    await page.getByRole('button', {name: /^Settings/}).click();
    await page.getByRole('button', {name: 'Connected agents'}).click();
    await expect(
      page.getByRole('heading', {name: 'Seal', exact: true}),
    ).toBeVisible();
    await page.reload();
    await expect(page.getByLabel('Desk sheet')).toHaveValue(/Should I keep/);
    const sheet = (await rpc('state')).current + '.md';
    expect(await readFile(join(folder, 'desk', sheet), 'utf8')).toContain(
      'Should I keep',
    );
    expect(
      await readFile(join(folder, 'scroll/deciding.md'), 'utf8'),
    ).toContain('I prefer an agenda');
    expect(
      (await readFile(join(folder, '.sumi/moments.db')))
        .subarray(0, 16)
        .toString(),
    ).not.toContain('SQLite format');
  } finally {
    core.stdin.end();
  }
});

test('first reading shows a recent reply and leaves older history for idle time', async ({
  page,
}) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    // Keep this whole conversation in the permanent evaluation split.
    let group = 'paper-review';
    while (createHash('sha256').update(group).digest()[0] >= 26) group += 'x';
    const turn = (sender: string, text: string, daysAgo: number) => ({
      sender,
      text,
      created_at: new Date(Date.now() - daysAgo * 86400000).toISOString(),
    });
    await writeFile(
      join(folder, 'conversations.json'),
      JSON.stringify([
        {
          uuid: group,
          chat_messages: [
            turn('assistant', 'Which typeface for the old poster?', 160),
            turn('human', 'Use the tall lettering.', 160),
            turn('assistant', 'Which paper for the new print?', 2),
            turn('human', 'Use the cream sheet.', 2),
          ],
        },
      ]),
    );
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await page.exposeFunction('sumiTestIPC', withTestModel(rpc));
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page.getByRole('button', {name: 'Think on this Mac instead'}).click();

    await expect(page.getByText('Ready to think on this Mac.')).toBeVisible();
    // Simulate a reading already requested on a previous build.
    await rpc('pickSource', {kind: 'export'});
    await rpc('beginFirstReading', {notify: false});
    await page.reload();
    await expect(
      page.getByText('Sumi hid 1 reply before making its guess.'),
    ).toBeVisible({timeout: 90000});
    await page
      .getByText('Which paper for the new print?', {exact: true})
      .click();
    await expect(
      page.getByText('You said · Use the cream sheet.'),
    ).toBeVisible();
    await expect(page.getByText(/will continue this reading/)).toBeVisible();
    await expect(
      page.getByText('Which typeface for the old poster?'),
    ).toHaveCount(0);
    await page.reload();
    await expect(page.getByText(/will continue this reading/)).toBeVisible();
  } finally {
    core.stdin.end();
  }
});

test('first reading survives a reloaded window and resumes a scored notebook after quit', async ({
  page,
  context,
}) => {
  let notebook = await fixtureNotebook();
  const {folder} = notebook;
  try {
    let group = 'evening-edits';
    while (createHash('sha256').update(group).digest()[0] >= 26) group += 'x';
    const choices = [
      ['Which pen for the cover?', 'Use the fine nib.'],
      ['When should we review the proof?', 'Tomorrow morning.'],
      ['What should the cover show?', 'One printed leaf.'],
      ['Where should the title go?', 'Below the picture.'],
      ['Which paper for the insert?', 'The cream sheet.'],
      ['How many proofs should we send?', 'Just one.'],
    ];
    await writeFile(
      join(folder, 'conversations.json'),
      JSON.stringify([
        {
          uuid: group,
          chat_messages: choices.flatMap(([question, answer]) => [
            {sender: 'assistant', text: question},
            {
              sender: 'human',
              text: answer,
              created_at: new Date(Date.now() - 86400000).toISOString(),
            },
          ]),
        },
      ]),
    );
    await notebook.rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await notebook.rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await context.exposeFunction('sumiTestIPC', (op: string, args: object) =>
      notebook.rpc(op, args),
    );
    await context.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await notebook.rpc('pickSource', {kind: 'export'});
    await notebook.rpc('beginFirstReading', {notify: false});
    await page.reload();
    await expect(
      page.getByRole('heading', {name: 'Reading your words.'}),
    ).toBeVisible();
    const job = (await notebook.rpc('firstReadingProgress')).job;
    await page.reload();
    await expect(
      page.getByRole('heading', {name: 'Reading your words.'}),
    ).toBeVisible();
    await notebook.rpc('beginFirstReading', {notify: false});
    expect((await notebook.rpc('firstReadingProgress')).job.started).toBe(
      job.started,
    );
    await expect
      .poll(async () => (await notebook.rpc('state')).scores.length, {
        timeout: 60000,
      })
      .toBeGreaterThan(0);
    const before = await notebook.rpc('state');
    expect(before.reading).toBeNull();
    expect(before.scores.length).toBeLessThan(choices.length);
    await page.close();
    const exited = new Promise<void>(resolve =>
      notebook.core.once('close', () => resolve()),
    );
    notebook.core.stdin.end();
    await exited;
    notebook = await fixtureNotebook(false, undefined, folder);
    const reopened = await context.newPage();
    await reopened.goto('/');
    await expect(
      reopened.getByRole('heading', {name: 'Reading your words.'}),
    ).toBeVisible();
    expect((await notebook.rpc('firstReadingProgress')).job.started).toBe(
      job.started,
    );
    await expect(
      reopened.getByText('Sumi hid 6 replies before making its guesses.'),
    ).toBeVisible({timeout: 90000});
    const after = await notebook.rpc('state');
    expect(after.firstReading).toBeNull();
    expect(after.scores).toHaveLength(choices.length);
    for (const scored of before.scores)
      expect(after.scores).toContainEqual(scored);
    await reopened.getByRole('button', {name: 'Open the Desk'}).click();
    await expect(reopened.getByLabel('Desk sheet')).toBeVisible();
  } finally {
    notebook.core.stdin.end();
  }
});

test('a requested export arrives after reopening and waits for consent', async ({
  page,
}) => {
  let notebook = await fixtureNotebook();
  const {folder} = notebook;
  const downloads = join(folder, 'Downloads');
  try {
    await mkdir(downloads);
    await notebook.rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await notebook.rpc('finish');
    await page.exposeFunction('sumiTestIPC', (op: string, args: object) =>
      notebook.rpc(op, args),
    );
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page.getByLabel('Desk sheet').fill('Keep this draft open.');
    await expect
      .poll(async () => (await notebook.rpc('state')).sheet)
      .toBe('Keep this draft open.');
    await page.keyboard.press('Meta+,');
    await page.getByText('Sources and readings', {exact: true}).click();
    await page.getByRole('button', {name: 'Request ChatGPT history ↗'}).click();
    await expect(
      page.getByText('Waiting for your chat export in Downloads.'),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Stop looking'}).click();
    await expect(page.getByRole('button', {name: 'Stop looking'})).toHaveCount(
      0,
    );
    await page.getByRole('button', {name: 'Request ChatGPT history ↗'}).click();
    await expect(
      page.getByRole('button', {name: 'Stop looking'}),
    ).toBeVisible();
    notebook.core.stdin.end();
    await new Promise<void>(resolve =>
      notebook.core.once('close', () => resolve()),
    );
    let group = 'print-review';
    while (createHash('sha256').update(group).digest()[0] < 52) group += 'x';
    const part = join(folder, 'conversations-01.json');
    await writeFile(
      part,
      JSON.stringify([
        {
          uuid: group,
          chat_messages: [
            {sender: 'assistant', text: 'Which paper feels right?'},
            {
              sender: 'human',
              text: 'The cream sheet.',
              created_at: new Date().toISOString(),
            },
          ],
        },
      ]),
    );
    const filename = 'cafe'.repeat(16) + '-history.ZIP';
    const archive = join(downloads, filename);
    execFileSync('/usr/bin/zip', ['-q', '-j', archive, part]);
    await rename(archive, archive + '.download');
    notebook = await fixtureNotebook(false, undefined, folder);
    await page.reload();
    await expect(page.getByLabel('Desk sheet')).toHaveValue(
      'Keep this draft open.',
    );
    const offer = page.getByRole('button', {
      name: `Add ${filename} to Sumi`,
    });
    await expect(offer).toHaveCount(0);
    await rename(archive + '.download', archive);
    // Returning to the window refreshes the notebook just as in the native app.
    await expect(async () => {
      await page.evaluate(() => window.dispatchEvent(new Event('focus')));
      await expect(offer).toBeVisible({timeout: 800});
    }).toPass({timeout: 15000});
    expect((await notebook.rpc('state')).sources).toEqual([]);
    expect((await notebook.rpc('read')).training).toBe(0);
    await page.setViewportSize({width: 520, height: 560});
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(520);
    await offer.click();
    await expect(
      page.getByText(`Added ${filename} to your sources.`),
    ).toBeVisible();
    await expect(page.getByLabel('Desk sheet')).toHaveValue(
      'Keep this draft open.',
    );
    await page.keyboard.press('Meta+,');
    await page.getByText('Sources and readings', {exact: true}).click();
    await page.getByRole('button', {name: 'Read selected sources'}).click();
    await expect
      .poll(async () => (await notebook.rpc('state')).reading?.training)
      .toBe(1);
    const state = await notebook.rpc('state');
    expect(state.requestedExport).toBeNull();
    expect(state.sources.map((s: {path: string}) => s.path)).toEqual([archive]);
  } finally {
    notebook.core.stdin.end();
  }
});

test('paper, brush and downloadable local Mac archive', async ({
  page,
}, info) => {
  await page.goto('/website/index.html');
  await expect(page.getByRole('heading', {name: 'Sumi'})).toBeVisible();
  await page.mouse.move(80, 80);
  await page.mouse.down();
  await page.mouse.move(165, 105, {steps: 12});
  await page.mouse.up();
  const painted = await page.locator('canvas').evaluate(el => {
    const canvas = el as HTMLCanvasElement;
    return canvas
      .getContext('2d')!
      .getImageData(0, 0, canvas.width, canvas.height)
      .data.some((n, i) => i % 4 === 3 && n > 0);
  });
  expect(painted).toBe(true);
  const download = page.waitForEvent('download');
  await page.getByRole('link', {name: /Download for Mac/}).click();
  const result = await download;
  expect(result.suggestedFilename()).toBe('Sumi-macOS.zip');
  expect(await result.failure()).toBeNull();
  const bytes = await readFile((await result.path())!);
  expect(bytes.subarray(0, 4).toString('hex')).toBe('504b0304');
  expect(bytes.includes(Buffer.from('Sumi.app/Contents/MacOS/sumi'))).toBe(
    true,
  );
  await page.emulateMedia({colorScheme: 'dark', reducedMotion: 'reduce'});
  await page.setViewportSize({width: 520, height: 480});
  await page.screenshot({
    path: info.outputPath('website-dark.png'),
    animations: 'disabled',
  });
});

test('Mirror separates paired blind results by prediction month and model', async ({
  page,
}) => {
  await page.clock.setFixedTime(new Date(2026, 6, 5, 12));
  await page.addInitScript(() => {
    const cases = [
      ['may-win', '2026-05-20', 'held-out', 'A', 0, 2],
      ['may-both', '2026-05-21', 'held-out', 'A', 2, 3],
      ['may-loss', '2026-05-23', 'held-out', 'B', 3, 0],
      ['may-other-grader', '2026-05-24', 'held-out', 'B', 2, 3],
      ['june-tie', '2026-06-04', 'held-out', 'A', 0, 1],
      ['live', '2026-06-04', 'live', 'A', 3, 3],
      ['unpaired', '2026-06-04', 'held-out', 'A', null, 2],
    ];
    const scores = cases.map(([id, t, scope, model, coldGrade, grade]) => ({
      id,
      predictedAt: t + 'T12:00:00Z',
      date: '2026-07-03T12:00:00Z',
      scope,
      model,
      grader: id === 'may-other-grader' ? 'Z' : undefined,
      coldGrade,
      grade,
      category: 'words',
      reflection: 'reviewed',
    }));
    const state = {
      setup: true,
      current: 'sheet',
      sheet: '',
      sheets: [],
      scores,
      log: [],
      entries: [
        {
          text: 'Sketch before choosing a typeface.',
          updated: '2026-07-04T12:00:00Z',
        },
        {text: 'Use an older idea.', updated: '2026-06-21T12:00:00Z'},
        {text: 'A later entry.', updated: '2026-07-06T12:00:00Z'},
      ],
      routes: [],
      tuning: [],
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: async (cmd: string, p: any) =>
        cmd === 'notebook' && p.operation === 'state' ? state : 1,
    };
  });
  await page.goto('/');
  await expect(page.getByLabel('Desk sheet')).toBeVisible();
  await page.keyboard.press('Meta+3');
  const note = page.getByRole('region', {name: 'Sunday note'});
  await expect(note).toContainText('one new or revised entry');
  await expect(note).toContainText('Sketch before choosing a typeface.');
  await expect(note).toContainText('5 of 7 scored guesses');
  await expect(note).not.toContainText('A later entry');
  await page.getByText('Does personalization help?').click();
  const table = page.getByRole('table', {
    name: 'Matched guesses by prediction month and model',
  });
  await expect(table.getByRole('row')).toHaveCount(5);
  await expect(table.getByRole('row').nth(1).getByRole('cell')).toHaveText([
    '0/1',
    '0/1',
    '0 pp',
  ]);
  await expect(table.getByRole('row').nth(2).getByRole('cell')).toHaveText([
    '1/2',
    '2/2',
    '+50 pp',
  ]);
  await expect(table.getByRole('row').nth(3).getByRole('cell')).toHaveText([
    '1/1',
    '0/1',
    '-100 pp',
  ]);
  await expect(table.getByRole('row').nth(4)).toContainText('Graded by Z');
  await expect(table.getByRole('row').nth(4).getByRole('cell')).toHaveText([
    '1/1',
    '1/1',
    '0 pp',
  ]);
  await expect(table.getByRole('rowheader').first()).toContainText('Jun');
});

test('Desk distinguishes accepted ink, a written alternative, dismissal and moving the caret', async ({
  page,
}, info) => {
  const {core, rpc} = await fixtureNotebook();
  const context = 'I think we can call';
  const hint = page.getByText('accepts the light ink', {exact: false});
  const sheet = page.getByLabel('Desk sheet');
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    const models = await rpc('models');
    const model = models.find((m: any) => m.name === 'llama3.2:3b');
    await rpc('model', model);
    await rpc('finish');
    for (let n = 0; n < 2; ) {
      const id = await rpc('newSheet');
      if (createHash('sha256').update(id).digest()[0] < 52) continue;
      await rpc('save', {
        id,
        text: 'What would you say at the end of this meeting?',
      });
      const sealed = await rpc('sealGuess', {id});
      await rpc('breakSeal', {
        id: sealed.id,
        actual: 'We can call this a wrap today.',
      });
      await rpc('verdict', {id: sealed.id, right: false});
      n++;
    }
    await rpc('newSheet');
    let hold = false;
    let release: (() => void) | undefined;
    await page.exposeFunction('sumiTestIPC', async (op: string, args: any) => {
      const result = await rpc(op, args);
      if (op === 'ghost' && hold)
        await new Promise<void>(r => {
          release = r;
        });
      return result;
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await sheet.evaluate(el => {
      el.addEventListener(
        'input',
        () => {
          const start = performance.now();
          const observer = new MutationObserver(() => {
            if (document.querySelector('.ghost-layer')) {
              el.setAttribute(
                'data-display-ms',
                String(Math.round(performance.now() - start)),
              );
              observer.disconnect();
            }
          });
          observer.observe(document.body, {childList: true, subtree: true});
        },
        {once: true},
      );
    });
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    console.log(
      'Desk input to visible ghost ink (ms)',
      await sheet.getAttribute('data-display-ms'),
    );
    await page.screenshot({
      path: info.outputPath('desk-writing.png'),
      animations: 'disabled',
    });
    await sheet.press('Tab');
    await expect(sheet).toHaveValue(context + ' this a wrap today.');
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    await sheet.press('ArrowLeft');
    await expect(hint).toHaveCount(0);
    await sheet.press('Meta+ArrowRight');
    await expect(hint).toBeVisible();
    await sheet.press('Escape');
    await expect(hint).toHaveCount(0);
    await sheet.fill('');
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    await sheet.pressSequentially(' it a rehearsal.', {delay: 12});
    await sheet.blur();
    await expect
      .poll(
        async () =>
          (await rpc('state')).scores.find((v: any) => v.status === 'edited')
            ?.actual,
      )
      .toBe('it a rehearsal.');
    // Discard an inference that returns after the writer selects earlier text.
    hold = true;
    await sheet.fill(context);
    await expect.poll(() => !!release).toBe(true);
    await sheet.press('ArrowLeft');
    hold = false;
    release!();
    await page.keyboard.press('Meta+3');
    await page.getByText('Details', {exact: true}).click();
    await page.getByText('How the score is measured').click();
    await expect(
      page.getByText('Ghost ink · 1 accepted of 4 shown.'),
    ).toBeVisible();
    await page.getByText('Prediction record', {exact: true}).click();
    await expect(page.getByText('You wrote · it a rehearsal.')).toBeVisible();
    const rows = (await rpc('state')).scores.filter(
      (v: any) => v.category === 'ghost',
    );
    expect(rows.map((r: any) => r.status).sort()).toEqual([
      'accepted',
      'dismissed',
      'edited',
      'shown',
    ]);
    expect(rows.every((r: any) => r.sources.includes('desk'))).toBe(true);
  } finally {
    core.stdin.end();
  }
});

test('morning return preserves the old sheet and delivers a waiting agent its answer', async ({
  page,
}) => {
  page.setDefaultTimeout(12000);
  const {core, rpc, folder} = await fixtureNotebook(true);
  const configPath = join(folder, 'agent-config', '.claude.json');
  const personal = {
    mcpServers: {sumi: {command: 'personal-reader'}},
    theme: 'ink',
  };
  await mkdir(join(folder, 'agent-config'), {recursive: true});
  await writeFile(configPath, JSON.stringify(personal));
  let agent: ReturnType<typeof spawn> | undefined;
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.clock.setFixedTime(new Date(2026, 9, 31, 21));
    await page.goto('/');
    const sheet = page.getByLabel('Desk sheet');
    const evening = 'I want to finish the sketch tomorrow.';
    await sheet.fill(evening);
    await expect.poll(async () => (await rpc('state')).sheet).toBe(evening);
    await page.keyboard.press('Meta+4');
    await page
      .getByRole('button', {name: 'Claude Code ＋', exact: true})
      .click();
    await page
      .getByRole('button', {name: 'Connect Claude Code', exact: true})
      .click();
    await page
      .locator('summary')
      .filter({hasText: 'Claude Code'})
      .first()
      .click();
    await page
      .getByLabel('Permission', {exact: true})
      .selectOption('Answer for me');
    await expect
      .poll(async () => (await rpc('state')).agents[0]?.permission)
      .toBe('Answer for me');
    await expect(
      page.getByLabel('Line for Claude Code’s instructions'),
    ).toHaveValue(/sumi_answer/);
    await expect(page.getByText('MCP server · sumi-2')).toBeVisible();
    const installed = await readFile(configPath, 'utf8');
    const config = JSON.parse(installed).mcpServers['sumi-2'];
    agent = spawn(config.command, config.args, {
      stdio: ['pipe', 'pipe', 'inherit'],
    });
    const lines = createInterface({input: agent.stdout!})[
      Symbol.asyncIterator
    ]();
    let serial = 0;
    async function ask(name: string, args: Record<string, string | string[]>) {
      agent!.stdin!.write(
        JSON.stringify({
          jsonrpc: '2.0',
          id: ++serial,
          method: 'tools/call',
          params: {name, arguments: args},
        }) + '\n',
      );
      const row = await lines.next();
      if (row.done) throw Error('Agent exited before receiving a result');
      const {result, error} = JSON.parse(row.value);
      if (error) throw Error(error.message);
      if (result.isError) throw Error(result.content[0].text);
      return JSON.parse(result.content[0].text);
    }
    const question = 'Which sketch should I prepare for review?';
    const ticket = await ask('sumi_answer', {question});
    expect(ticket.queued).toBe(true);
    for (let i = 0; i < 64; i++)
      await ask('sumi_context', {task: 'Look up review preferences.'});
    expect((await ask('sumi_answer', {ticket: ticket.id})).queued).toBe(true);
    await page.clock.setFixedTime(new Date(2026, 10, 1, 8));
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await expect(sheet).toHaveValue('');
    await expect(
      page.getByRole('button', {
        name: /1 question from Claude Code is waiting/,
      }),
    ).toBeVisible();
    await sheet.fill('Start with the ink sketch.');
    await expect
      .poll(async () => (await rpc('state')).sheet)
      .toBe('Start with the ink sketch.');
    await page.reload();
    await expect(sheet).toHaveValue('Start with the ink sketch.');
    await page
      .getByRole('button', {name: /1 question from Claude Code/})
      .click();
    const answerButton = page.getByRole('button', {
      name: 'Answer question',
      exact: true,
    });
    await expect(answerButton).toBeFocused();
    await answerButton.click();
    await page.getByLabel('Your answer').fill('Use the ink sketch.');
    await page.getByRole('button', {name: 'Keep answer'}).click();
    await expect(
      page.getByText('Use the ink sketch.', {exact: true}),
    ).toBeVisible();
    expect(await ask('sumi_answer', {ticket: ticket.id})).toMatchObject({
      answer: 'Use the ink sketch.',
      decidedBy: 'user',
    });
    await page.keyboard.press('Meta+1');
    await expect(
      page.getByRole('button', {name: /is waiting for you/}),
    ).toHaveCount(0);
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: new RegExp(evening)}).click();
    await expect(sheet).toHaveValue(evening);
    await page.reload();
    await expect(sheet).toHaveValue(evening);
    expect((await rpc('state')).sheets).toHaveLength(2);
    await page.keyboard.press('Meta+4');
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await page.keyboard.press('Meta+3');
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
    await page.keyboard.press('Alt+Space');
    await expect(sheet).toHaveValue('');
    expect((await rpc('state')).sheets).toHaveLength(3);
    await test.step('review earned automatic answers and change this agent’s confidence threshold', async () => {
      const request = {
        question: 'Which format do you prefer?',
        options: ['Bullets', 'Prose'],
      };
      let qualified = 0;
      let automatic = 0;
      for (let i = 0; i < 90; i++) {
        const reply = await ask('sumi_answer', request);
        if (!reply.queued) {
          expect(reply.answer).toBe('Bullets');
          automatic++;
          if (reply.confidence >= 0.97 && ++qualified === 20) break;
        }
        await rpc('respond', {id: reply.id, answer: 'Bullets'});
      }
      expect(qualified).toBe(20);
      await page.keyboard.press('Meta+4');
      const agentSection = page
        .locator('details')
        .filter({has: page.getByLabel('Permission', {exact: true})});
      await agentSection.locator('summary').first().click();
      await expect(
        agentSection.getByRole('button', {name: 'Raise minimum to 97%'}),
      ).toHaveCount(0);
      const question = page
        .locator('summary')
        .filter({hasText: request.question})
        .first();
      await question.click();
      await page
        .getByRole('button', {name: 'That was right', exact: true})
        .click();
      await expect(agentSection).toContainText(
        automatic +
          ' of ' +
          automatic +
          ' automatic answers reviewed · 0 corrected.',
      );
      await agentSection
        .getByRole('button', {name: 'Raise minimum to 97%'})
        .click();
      await expect
        .poll(async () => (await rpc('state')).agents[0].threshold)
        .toBe(0.97);
    });
    await writeFile(configPath, 'unfinished json');
    await page.getByRole('button', {name: 'Revoke access'}).click();
    await page.getByRole('button', {name: 'Confirm revocation'}).click();
    await expect(
      page.getByText('Access revoked. Configuration cleanup is pending.'),
    ).toBeVisible();
    await expect(
      ask('sumi_context', {task: 'Check a preference.'}),
    ).rejects.toThrow('not connected');
    await expect(readFile(config.args.at(-1))).rejects.toMatchObject({
      code: 'ENOENT',
    });
    expect(await readFile(configPath, 'utf8')).toBe('unfinished json');
    await writeFile(configPath, installed);
    await page.getByRole('button', {name: 'Finish removal'}).click();
    await expect(
      page.getByRole('button', {name: 'Claude Code ＋'}),
    ).toBeVisible();
    expect(JSON.parse(await readFile(configPath, 'utf8'))).toEqual(personal);
  } finally {
    agent?.stdin?.end();
    core.stdin.end();
  }
});

test('three weeks of observed choices lead to one user-approved revision', async ({
  page,
}) => {
  const text = 'I decline all recurring meetings.';
  const rule = {
    id: 'recurring',
    section: 'deciding',
    authored: true,
    confidence: 1,
    evidence: [],
    sources: [],
  };
  const revision = createHash('sha256').update(text).digest('hex');
  const markdown =
    '# How you decide\n\n## Recurring meetings\n' +
    text +
    '\n<!-- sumi ' +
    JSON.stringify(rule) +
    ' -->\n';
  const scores = [21, 10, 1].map((days, index) => ({
    id: 'case-' + index,
    date: new Date(Date.now() - days * 86400000).toISOString(),
    category: 'decisions',
    scope: 'live',
    grade: 0,
    question: [
      'Join a monthly design review?',
      'Attend the weekly critique?',
      'Accept the recurring studio invite?',
    ][index],
    guess: 'I would decline.',
    actual: [
      'Join the monthly review.',
      'Attend the weekly critique.',
      'Accept the studio invite.',
    ][index],
    ruleIds: ['recurring'],
    ruleRevisions: {recurring: revision},
    ruleCheck: {id: 'recurring', revision},
  }));
  const {rpc, core, folder} = await fixtureNotebook(false, {markdown, scores});
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.keyboard.press('Meta+3');
    await expect(page.getByText(/possible exceptions on 3 days/)).toBeVisible();
    await page.getByText('Read the 3 observations').click();
    await expect(
      page
        .locator('blockquote')
        .filter({hasText: 'Attend the weekly critique.'}),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Change or exception?'}).click();
    const revised = 'I make time for recurring studio critiques.';
    await page
      .getByLabel('How would you word the change or exception?')
      .fill(revised);
    await page.getByRole('button', {name: 'A change', exact: true}).click();
    await expect(page.getByRole('heading', {name: 'Surprises'})).toHaveCount(0);
    await page.keyboard.press('Meta+2');
    await expect(page.getByRole('button', {name: revised})).toBeVisible();
    expect(
      await readFile(join(folder, 'scroll/deciding.md'), 'utf8'),
    ).toContain(revised);
    const state = await rpc('state');
    expect(state.scores.map((v: any) => v.grade)).toEqual([0, 0, 0]);
    expect(
      state.scores.every((v: any) => v.reflection === revised),
    ).toBeTruthy();
  } finally {
    core.stdin.end();
  }
});

test('Desk matches approved goals to the active paragraph and remembers a dismissal', async ({
  page,
}) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    const first = await rpc('create', {
      section: 'goals',
      text: 'Read about studio lighting before buying equipment.',
    });
    const strongest = 'Practice studio lighting with portraits.';
    const id = 'a'.repeat(32);
    const goals = join(folder, 'scroll/goals.md');
    await writeFile(
      goals,
      (await readFile(goals, 'utf8')) +
        '\n## Portraits\n' +
        strongest +
        '\n<!-- sumi ' +
        JSON.stringify({
          id,
          status: 'stamped',
          confidence: 0.95,
          importance: 1,
          updated: new Date().toISOString(),
        }) +
        ' -->\n',
    );
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.clock.install();
    await page.goto('/');
    const sheet = page.getByLabel('Desk sheet');
    const note = page.getByRole('complementary', {
      name: 'Goal from your Scroll',
    });
    await sheet.fill('Think about next week before bed.');
    await page.clock.runFor(900);
    await expect(note).toHaveCount(0);
    await sheet.fill('Studios and lightings are plurals.');
    await page.clock.runFor(900);
    await expect(note).toHaveCount(0);
    const related = 'Adjust studio lighting for portraits.';
    await sheet.fill(related + '\nA grocery list for Sunday.');
    await page.clock.runFor(900);
    await expect(note).toHaveCount(0);
    await sheet.press('Meta+ArrowUp');
    await expect(note).toContainText(strongest);
    await note.getByRole('button', {name: 'Dismiss margin note'}).click();
    await expect(
      page.getByText('Margin notes quiet for 24 hours.'),
    ).toBeVisible();
    await page.reload();
    await sheet.fill('Read about studio lighting before buying equipment.');
    await page.clock.runFor(900);
    await expect(note).toHaveCount(0);
    const muted = (await rpc('state')).marginMuted;
    await page.clock.setFixedTime(
      new Date(Date.parse(muted.quietUntil) + 1000),
    );
    await sheet.fill('Adjust studio lighting before buying equipment.');
    await expect(note).toContainText(first.text);
    await sheet.fill(related);
    await expect(note).toContainText(first.text);
  } finally {
    core.stdin.end();
  }
});

test('resume the publisher’s real GGUF download and seal a decision without an external runtime', async ({
  page,
}) => {
  test.skip(
    process.env.SUMI_TEST_MODEL_DOWNLOAD !== '1',
    'Requires the hardware-tier GGUF download.',
  );
  test.setTimeout(1800000);
  page.setDefaultTimeout(15000);
  const first = await fixtureNotebook();
  const partial = join(
    first.folder,
    '.sumi',
    'models',
    (await first.rpc('modelDownloadStatus')).file + '.partial',
  );
  let writer: typeof first | undefined, reader: typeof first | undefined;
  const closed = (core: typeof first.core) =>
    new Promise<void>(resolve => {
      if (core.exitCode !== null || core.signalCode !== null) resolve();
      else core.once('exit', () => resolve());
    });
  try {
    await first.rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    // A verified local copy can seed the prefix on repeat runs. Both continuations
    // still use the publisher, and the runtime verifies the entire finished file.
    let startingBytes = 0;
    const seed = process.env.SUMI_TEST_GGUF_PREFIX;
    if (seed && basename(seed) === basename(partial, '.partial')) {
      const bytes = (await stat(seed)).size;
      startingBytes = Math.max(0, bytes - 96 * 1024 * 1024);
      await mkdir(dirname(partial), {recursive: true});
      await copyFile(seed, partial, constants.COPYFILE_FICLONE);
      await truncate(partial, startingBytes);
    }
    first.core.stdin.write(
      JSON.stringify({operation: 'downloadBundled', args: {}}) + '\n',
    );
    await expect
      .poll(async () => (await stat(partial).catch(() => null))?.size ?? 0, {
        timeout: 120000,
        intervals: [100],
      })
      .toBeGreaterThan(startingBytes + 4_000_000);
    first.core.kill('SIGTERM');
    await closed(first.core);
    const offset = (await stat(partial)).size;
    writer = await fixtureNotebook(false, undefined, first.folder);
    reader = await fixtureNotebook(false, undefined, first.folder);
    const steps: number[] = [];
    await page.exposeFunction('sumiTestIPC', async (op: string, args: any) => {
      if (op === 'models') return []; // Model a Mac with no exposed Ollama or LM Studio.
      if (op === 'modelDownloadStatus') {
        const progress = await reader!.rpc(op, args);
        steps.push(progress.bytes);
        return progress;
      }
      try {
        return await writer!.rpc(op, args);
      } catch (e) {
        if (op === 'downloadBundled') console.error('Bundled download:', e);
        throw e;
      }
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await expect(
      page.getByRole('heading', {name: /Your notebook stays/}),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Think on this Mac instead'}).click();
    await page.getByRole('button', {name: /^Resume .* download$/}).click();
    await expect(
      page.getByRole('progressbar', {name: 'Local model download'}),
    ).toBeVisible();
    await expect(page.getByText('Ready to think on this Mac.')).toBeVisible({
      timeout: 1500000,
    });
    expect(steps).toContain(offset);
    expect(steps.some(n => n > offset)).toBeTruthy();
    await page.getByRole('button', {name: 'Continue', exact: true}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.reload();
    await page
      .getByLabel('Desk sheet')
      .fill('Should I sketch the cover first or start painting right away?');
    await page.getByRole('button', {name: 'Seal a guess'}).click();
    await page.getByRole('button', {name: 'Open sealed guess'}).click();
    await page
      .getByLabel('Your decision')
      .fill('I will sketch the cover first.');
    await page.getByRole('button', {name: 'Break the seal'}).click();
    const state = await writer.rpc('state');
    expect(state.model.runtime).toBe('Sumi');
    expect(state.sealed.guess.trim().length).toBeGreaterThan(0);
    console.log(
      'Real bundled download resumed at',
      offset,
      'bytes. Model:',
      state.model.name,
    );
  } finally {
    for (const session of [first, reader, writer]) {
      if (!session) continue;
      if (session.core.exitCode === null && session.core.signalCode === null)
        session.core.stdin.end();
      await closed(session.core);
    }
    await rm(first.folder, {recursive: true, force: true});
  }
});

test('the ChatGPT choice waits for a completed check and keeps advanced setup in Settings', async ({
  page,
}, info) => {
  const {rpc, core} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0.1, 0.56],
        [0.18, 0.43],
        [0.28, 0.35],
        [0.39, 0.36],
        [0.52, 0.46],
        [0.67, 0.59],
        [0.78, 0.6],
        [0.9, 0.48],
      ],
    });
    let attempts = 0;
    let connected = false;
    let finishCheck: (() => void) | undefined;
    await page.exposeFunction('sumiTestIPC', async (op: string, args: any) => {
      // Browser automation cannot consent on a person's OpenAI account.
      // The native transport test checks the actual HTTP and streaming contract.
      if (op === 'connectPlan') {
        if (++attempts === 1) throw Error('Sign-in was cancelled.');
        await new Promise<void>(resolve => {
          finishCheck = resolve;
        });
        connected = true;
        return true;
      }
      const value = await rpc(op, args);
      if (op === 'state' && connected)
        return {
          ...value,
          model: {runtime: 'ChatGPT', name: 'plan-test'},
          providers: [{id: 'ChatGPT', primary: true}],
        };
      return value;
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    const signIn = page.getByRole('button', {name: 'Continue with ChatGPT'});
    await expect(signIn).toBeVisible();
    await expect(
      page.getByRole('button', {name: 'Think on this Mac instead'}),
    ).toBeVisible();
    await expect(page.getByRole('combobox')).toHaveCount(0);
    await page.screenshot({path: info.outputPath('think.png')});
    await signIn.click();
    await expect(page.getByRole('alert')).toContainText('cancelled');
    await signIn.click();
    await expect(page.getByText('Waiting for ChatGPT…')).toBeVisible();
    await expect(
      page.getByRole('button', {name: 'Continue', exact: true}),
    ).toHaveCount(0);
    finishCheck!();
    await expect(page.getByRole('status')).toHaveText(
      'Connected to your ChatGPT plan.',
    );
    await page.getByRole('button', {name: 'Continue', exact: true}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.reload();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.keyboard.press('Meta+,');
    await expect(
      page.getByRole('heading', {name: 'Settings', exact: true}),
    ).toBeVisible();
    await expect(
      page.getByRole('button', {name: 'Disconnect ChatGPT'}),
    ).toBeVisible();
    await page.getByText('Optional providers', {exact: true}).click();
    await expect(page.getByRole('combobox')).toBeVisible();
    await page.getByRole('button', {name: 'Done'}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
  } finally {
    core.stdin.end();
  }
});

test('one sheet keeps a conversation, a correction, and a reusable method', async ({
  page,
}, info) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0.14, 0.65],
        [0.48, 0.23],
        [0.7, 0.71],
        [0.9, 0.47],
      ],
    });
    await rpc('model', {name: 'llama3.2:3b', runtime: 'Ollama'});
    await rpc('finish');
    await rpc('create', {
      section: 'deciding',
      text: 'For museum visits, I pick one painting to spend time with.',
    });
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    const field = page.getByLabel('Desk sheet');
    async function ask(text: string, count: number) {
      await field.fill(text);
      await page.getByRole('button', {name: 'Ask Sumi', exact: true}).click();
      await expect(page.locator('.exchange')).toHaveCount(count, {
        timeout: 90000,
      });
      await expect(field).toHaveValue('');
    }
    await ask(
      'Help me plan a museum visit. I have twenty minutes to see a painting called Blue Heron. Give me two short steps.',
      1,
    );
    await page.getByText('Behind this reply', {exact: true}).click();
    await expect(page.locator('.receipt')).toContainText('For museum visits');
    await page.screenshot({path: info.outputPath('conversation.png')});
    const first = await rpc('state');
    expect(
      await readFile(
        join(folder, 'desk', 'conversations', first.current + '.md'),
        'utf8',
      ),
    ).toContain(first.chat.turns[0].reply);
    await page.reload();
    await ask('Remind me of the name of the painting I mentioned.', 2);
    expect((await rpc('state')).chat.turns[1].reply).toMatch(/Blue Heron/i);
    await ask(
      'Help me plan a museum visit again, this time with five minutes to look at the painting.',
      3,
    );
    await ask(
      'For every museum visit, always choose one painting and give me a question to think about.',
      4,
    );
    const skill = (await rpc('state')).skills[0];
    expect(skill.title.trim()).toBeTruthy();
    expect(
      await readFile(join(folder, 'scroll', 'skills', skill.file), 'utf8'),
    ).toContain('one painting');
    await page.keyboard.press('Meta+2');
    await page.getByText(skill.title, {exact: true}).click();
    await expect(page.getByText(skill.body, {exact: true})).toBeVisible();
    await page.screenshot({path: info.outputPath('scroll-skills.png')});
    await page.keyboard.press('Meta+1');
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: /New sheet/}).click();
    await expect(page.locator('.exchange')).toHaveCount(0);
    await ask('Give me a plan for a museum visit tomorrow.', 1);
    const next = await rpc('state');
    expect(next.current).not.toBe(first.current);
    expect(next.chat.turns[0].skill).toBe(skill.title);
    expect(next.chat.private).toBe(true);
    await page.getByText('Behind this reply', {exact: true}).click();
    await expect(page.locator('.receipt')).toContainText(
      'Skill · ' + skill.title,
    );
    await page.emulateMedia({colorScheme: 'dark'});
    await page.screenshot({path: info.outputPath('night-ink.png')});
    await page.setViewportSize({width: 520, height: 600});
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(520);
    await page.getByRole('button', {name: 'Correct this reply'}).click();
    await expect(page.getByText('The reply you’re correcting')).toBeVisible();
    await expect(page.getByLabel('Your correction')).toBeVisible();
  } finally {
    core.stdin.end();
  }
});

test('a dinner question chooses its own tools and remembers a change of mind', async ({
  page,
}, info) => {
  const {core, rpc} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0.12, 0.6],
        [0.43, 0.25],
        [0.63, 0.72],
        [0.88, 0.4],
      ],
    });
    await rpc('model', {name: 'llama3.2:3b', runtime: 'Ollama'});
    await rpc('finish');
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    const field = page.getByLabel('Desk sheet');
    async function say(text: string) {
      await field.fill(text);
      await field.press('Enter');
      await expect(page.locator('.exchange')).toHaveCount(1, {timeout: 90000});
      await expect(field).toHaveValue('');
    }
    async function fresh() {
      await page.getByRole('button', {name: /^Sheets/}).click();
      await page.getByRole('button', {name: /New sheet/}).click();
      await expect(page.locator('.exchange')).toHaveCount(0);
    }
    const before =
      'When we split a dinner bill, always give me just the amount per person.';
    const after =
      'For dinner bills, from now on show the calculation as well as the amount per person.';
    await say(before);
    const first = (await rpc('state')).skills[0];
    expect(first.title.trim()).toBeTruthy();
    expect(first.body).toBe(before);
    await fresh();
    await say(after);
    const updated = (await rpc('state')).skills;
    expect(updated).toHaveLength(1);
    expect(updated[0].id).toBe(first.id);
    expect(updated[0].body).toBe(after);
    await page.reload();
    await fresh();
    await say(
      'Can you split a $96 dinner bill between four people and tell me what day tomorrow is?',
    );
    const turn = (await rpc('state')).chat.turns[0];
    const tomorrow = new Date(turn.at);
    tomorrow.setDate(tomorrow.getDate() + 1);
    const weekday = tomorrow.toLocaleDateString('en-US', {weekday: 'long'});
    const expected = tomorrow.toLocaleDateString('en-CA');
    expect(turn.tools.find((v: any) => v.tool === 'Calendar').date).toBe(
      expected,
    );
    expect(turn.tools.find((v: any) => v.tool === 'Calculator').value).toBe(24);
    expect(turn.skill).toBe(updated[0].title);
    await expect(page.locator('.reply')).toContainText('24');
    await expect(page.locator('.reply')).toContainText(weekday);
    await expect(page.getByLabel('Working')).toContainText('96/4 = 24');
    await page.screenshot({path: info.outputPath('dinner.png')});
    await page.getByText('Behind this reply', {exact: true}).click();
    await expect(page.locator('.receipt')).toContainText(updated[0].title);
    await fresh();
    await say(
      'What did I originally ask you to do with dinner bills, and how did that change?',
    );
    const timeline = page.getByLabel('Your earlier instructions');
    await expect(timeline.locator('li')).toHaveCount(2);
    await expect(timeline.locator('li').first()).toContainText(before);
    await expect(timeline.locator('li').last()).toContainText(after);
    await page.screenshot({path: info.outputPath('timeline.png')});
    await fresh();
    await say('My pottery budget is 84 dollars.');
    await fresh();
    await say(
      'Using the pottery budget I mentioned, how much can I spend per class if I take four classes?',
    );
    const budget = (await rpc('state')).chat.turns[0];
    const passages = budget.tools.find(
      (v: any) => v.tool === 'Notebook',
    ).passages;
    expect(passages.map((v: any) => v.words)).toEqual([
      'My pottery budget is 84 dollars.',
    ]);
    expect(budget.tools.find((v: any) => v.tool === 'Calculator')?.value).toBe(
      21,
    );
    await expect(page.locator('.reply')).toContainText('21');
  } finally {
    core.stdin.end();
  }
});

test('a weekend plan uses live sources and the current preference from another sheet', async ({
  page,
}, info) => {
  test.skip(
    process.env.SUMI_LIVE_SEARCH !== '1',
    'Uses the free public search service and an installed local model.',
  );
  const {core, rpc} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0.12, 0.75],
        [0.48, 0.35],
        [0.85, 0.52],
      ],
    });
    await rpc('model', {name: 'llama3.2:3b', runtime: 'Ollama'});
    await rpc('finish');
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).sumiTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    const field = page.getByLabel('Desk sheet');
    async function say(text: string) {
      await field.fill(text);
      await field.press('Enter');
      await expect(field).toHaveValue('', {timeout: 90000});
    }
    await say(
      'I live in Boston. On weekends I love hands-on printmaking workshops.',
    );
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: /New sheet/}).click();
    await say('What should I do this weekend?');
    const proposal = page.locator('.search-proposal');
    await expect(proposal).toContainText('Boston');
    await expect(proposal).toContainText('art workshops');
    const draft = (await rpc('state')).chat.turns[0];
    expect(draft.web.sources).toBeUndefined();
    await page
      .getByRole('button', {name: 'Search the web', exact: true})
      .click();
    await expect(page.locator('.source-link').first()).toBeVisible({
      timeout: 90000,
    });
    const result = (await rpc('state')).chat.turns[0];
    expect(result.web.status).toBe('done');
    expect(result.web.sources[result.web.pick].url).toMatch(/^https?:\/\//);
    expect(result.reply.length).toBeGreaterThan(30);
    await page.getByLabel('Show remembered preference').hover();
    await expect(page.getByLabel('Remembered words')).toContainText(
      'printmaking',
    );
    await page.screenshot({path: info.outputPath('weekend.png')});
    await page.getByRole('button', {name: 'Not true anymore'}).click();
    await page.reload();
    await expect(page.locator('.source-link').first()).toBeVisible();
    await page.keyboard.press('Meta+k');
    await page.getByText('Earlier memories', {exact: true}).click();
    await expect(page.locator('.dated-notes')).toContainText('printmaking');
    await page.keyboard.press('Meta+,');
    await page.getByText('Web search', {exact: true}).click();
    await page.getByLabel('Automatically look up weekend listings').check();
    await page.keyboard.press('Meta+1');
    async function fresh() {
      await page.getByRole('button', {name: /^Sheets/}).click();
      await page.getByRole('button', {name: /New sheet/}).click();
    }
    await fresh();
    await say('What should I do this weekend?');
    const automatic = (await rpc('state')).chat.turns[0];
    expect(automatic.web.status).toBe('done');
    expect(automatic.web.query).not.toMatch(/printmaking|art workshops/i);
    expect(automatic.usedMemory).toBeUndefined();
    await fresh();
    await page.getByLabel('Keep this sheet offline').check();
    await say('What should I do this weekend?');
    const offline = (await rpc('state')).chat.turns[0];
    expect(offline.web.status).toBe('private');
    expect(offline.web.sources).toBeUndefined();
    await expect(page.locator('.source-list, .search-proposal')).toHaveCount(0);
    console.log(
      'Weekend recommendation:',
      result.reply,
      'Source:',
      result.web.sources[result.web.pick].url,
    );
  } finally {
    core.stdin.end();
  }
});

test('read a workshop schedule, follow up from the file, then correct a dated project memory', async ({
  page,
}, info) => {
  let chunks = 0;
  const {rpc, core} = await fixtureNotebook(false, undefined, undefined, m => {
    chunks++;
    void page
      .evaluate(message => (window as any).sumiDelta?.(message.payload), m)
      .catch(() => {});
  });
  try {
    await rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    await page.exposeFunction('sumiTestIPC', rpc);
    await page.addInitScript(() => {
      const callbacks = new Map();
      let serial = 0;
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: (fn: any) => {
          callbacks.set(++serial, fn);
          return serial;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: (cmd: string, p: any) => {
          if (cmd === 'notebook')
            return (window as any).sumiTestIPC(p.operation, p.args);
          if (cmd === 'plugin:event|listen' && p.event === 'reply-delta')
            (window as any).sumiDelta = (payload: any) =>
              callbacks.get(p.handler)?.({
                event: p.event,
                payload,
                id: p.handler,
              });
          return Promise.resolve(1);
        },
      };
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: () => {},
      };
    });
    await page.goto('/');
    const field = page.getByLabel('Desk sheet');
    await page.getByLabel('Choose attachments').setInputFiles({
      name: 'studio-classes.csv',
      mimeType: 'text/csv',
      buffer: Buffer.from(
        'Workshop,Day,Start,Seats\nLinocut,Saturday,10 am,8\nBookbinding,Sunday,2 pm,6',
      ),
    });
    await expect(
      page.getByRole('button', {name: 'Remove studio-classes.csv'}),
    ).toBeVisible();
    await field.fill('Which workshop in my file is on Sunday?');
    await field.press('Enter');
    await expect(
      page.locator('.exchange').last().locator('.reply'),
    ).toContainText(/bookbinding/i, {timeout: 90000});
    expect(chunks).toBeGreaterThan(1);
    await page.getByRole('button', {name: 'Maybe later'}).click();
    await field.fill('How many seats does that Sunday class have?');
    await field.press('Enter');
    await expect(page.locator('.exchange')).toHaveCount(2, {timeout: 90000});
    await expect(
      page.locator('.exchange').last().locator('.reply'),
    ).toContainText(/6|six/i);
    await page.screenshot({path: info.outputPath('workshop.png')});
    await page.locator('.receipt summary').last().click();
    await page.getByRole('button', {name: 'Try this answer again'}).click();
    await expect(
      page.getByRole('button', {name: 'Try this answer again'}),
    ).toBeEnabled({timeout: 90000});
    expect((await rpc('state')).chat.turns[1].regenerations).toBe(1);
    await field.fill('I am working on the spring exhibition.');
    await field.press('Enter');
    await expect(page.locator('.exchange')).toHaveCount(3, {timeout: 90000});
    await page.keyboard.press('Meta+k');
    await page
      .getByText('I am working on the spring exhibition', {exact: true})
      .click();
    await page.getByRole('button', {name: 'Edit memory'}).click();
    await page
      .getByLabel('Correct this memory')
      .fill('I am working on the autumn exhibition.');
    await page.getByRole('button', {name: 'Save correction'}).click();
    await expect(
      page.getByText('I am working on the autumn exhibition.', {exact: true}),
    ).toBeVisible();
    await page.reload();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.keyboard.press('Meta+k');
    await expect(
      page.getByText('I am working on the autumn exhibition.', {exact: true}),
    ).toBeVisible();
    await page.keyboard.press('Alt+Space');
    await expect(field).toBeVisible();
    await field.fill('What did you say about bookbinding in our previous chat?');
    await field.press('Enter');
    const recalled = page.locator('.exchange').last();
    await expect(recalled.locator('.reply')).toContainText(/bookbinding/i, {timeout: 90000});
    await recalled.locator('.receipt summary').click();
    const passage = recalled
      .locator('.receipt blockquote')
      .filter({hasText: /bookbinding/i})
      .first();
    await expect(passage).toContainText('Assistant ·');
    await page.keyboard.press('Meta+,');
    await page.getByText('Your notebook', {exact: true}).click();
    await page.getByRole('button', {name: 'Review local measures'}).click();
    await expect(
      page.getByRole('columnheader', {name: 'Rejected'}),
    ).toBeVisible();
    const download = await rpc('exportNotebook');
    try {
      expect(
        execFileSync('unzip', ['-p', download, 'records/note.json'], {
          encoding: 'utf8',
        }),
      ).toContain('autumn exhibition');
      expect(
        execFileSync('unzip', ['-Z1', download], {encoding: 'utf8'}),
      ).not.toContain('.sumi/');
    } finally {
      await rm(download);
    }
  } finally {
    core.stdin.end();
  }
});
