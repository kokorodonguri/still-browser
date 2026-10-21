#!/usr/bin/env python3
"""Repeat real-browser operations and collect unforced jemalloc reports.

Requires Selenium and geckodriver. Use a dedicated test profile, no personal data.
Reported operation times include WebDriver overhead, not hardware input latency.
"""
import argparse
from datetime import datetime, timezone
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
from threading import Thread
import time

from benchmark_metrics import stats, assess_report

from selenium import webdriver
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service
from selenium.webdriver.support.ui import WebDriverWait


class Pages(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        body = ('<!doctype html><title>Still benchmark</title><style>p{height:100px}</style>'
                '<script>window.addEventListener("pageshow",e=>window.wasPersisted=e.persisted)</script>'
                + ''.join(f'<p>Repeatable scrolling fixture {i}</p>' for i in range(300))).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)




REPORT = r"""
const done = arguments[arguments.length - 1];
(async () => {
  const before = await ChromeUtils.requestProcInfo();
  const expected = [before.pid, ...before.children.map(p => p.pid)];
  const reports = [];
  let finished = false;
  const finish = async error => {
    if (finished) return;
    finished = true;
    clearTimeout(timer);
    let after = [];
    try {
      const info = await ChromeUtils.requestProcInfo();
      after = [info.pid, ...info.children.map(p => p.pid)];
    } catch (e) { error = String(e); }
    done({expected, after, reports, error, processTypes: before.children.map(p => ({pid:p.pid, type:p.type}))});
  };
  const timer = setTimeout(() => finish('memory report timed out'), 15000);
  const mgr = Cc['@mozilla.org/memory-reporter-manager;1'].getService(Ci.nsIMemoryReporterManager);
  try {
    mgr.getReports((process, path, kind, units, amount) => {
      if (path.startsWith('heap/') || ['resident', 'page-faults-soft', 'page-faults-hard'].includes(path)) {
        const match = process.match(/pid[= ]+(\d+)/);
        reports.push({pid: process ? (match ? Number(match[1]) : null) : before.pid,
                      process, path, units, amount});
      }
    }, null, () => finish(null), null, true);
  } catch (e) { finish(String(e)); }
})().catch(e => done({error:String(e), expected:[], after:[], reports:[]}));
"""




def file_hash(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(args):
    server = ThreadingHTTPServer(('127.0.0.1', 0), Pages)
    Thread(target=server.serve_forever, daemon=True).start()
    urls = json.loads(args.urls.read_text()) if args.urls else [f'http://127.0.0.1:{server.server_port}/{i}' for i in range(10)]
    if len(urls) < 2:
        raise ValueError('At least two URLs required')
    options = Options()
    options.binary_location = str(args.firefox.resolve())
    if args.headless:
        options.add_argument('-headless')
    options.set_preference('browser.shell.checkDefaultBrowser', False)
    options.set_preference('still.memory.autoUnload.enabled', args.auto_unload)
    service = Service(str(args.geckodriver.resolve()), env=dict(os.environ, MOZ_REMOTE_ALLOW_SYSTEM_ACCESS='1'))
    result = {'build': args.build, 'platform': platform.platform(), 'machine': platform.machine(),
              'headless': args.headless, 'autoUnload': args.auto_unload, 'fixture': not bool(args.urls),
              'urls': urls if args.urls else ['fixture/' + str(i) for i in range(10)],
              'engineSha256': file_hash(args.firefox),
              'xulSha256': file_hash(args.firefox.parent / ('xul.dll' if os.name == 'nt' else 'libxul.so')),
              'operations': [], 'allocator': [], 'errors': []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    driver = None
    try:
        driver = webdriver.Firefox(options=options, service=service)
        driver.set_script_timeout(25)
        driver.set_page_load_timeout(60)
        driver.set_window_size(1280, 900)

        def painted():
            driver.execute_async_script('const done=arguments[0]; requestAnimationFrame(()=>requestAnimationFrame(()=>done(true)));')

        handles = []
        expected_urls = {}
        for i, url in enumerate(urls):
            if i:
                driver.switch_to.new_window('tab')
            driver.get(url)
            painted()
            handles.append(driver.current_window_handle)
            expected_urls[handles[-1]] = driver.current_url

        def measure(operation, iteration, action):
            started = time.perf_counter()
            try:
                detail = action()
                result['operations'].append({'operation': operation, 'iteration': iteration,
                    'ms': (time.perf_counter() - started) * 1000, 'detail': detail, 'complete': True})
            except Exception as error:
                result['operations'].append({'operation': operation, 'iteration': iteration,
                    'ms': None, 'complete': False, 'error': str(error)})
                raise

        for iteration in range(args.repeats):
            # Keep setup/navigation outside the back and switch timing windows.
            def switch():
                handle = handles[iteration % len(handles)]
                driver.switch_to.window(handle)
                WebDriverWait(driver, 60).until(lambda d: d.current_url == expected_urls[handle] and
                    d.execute_script("return document.readyState") == "complete")
                painted()
            measure('tab-switch', iteration, switch)
            measure('page-load', iteration, lambda: (driver.get(urls[0]), painted()))
            driver.get(urls[1])
            painted()
            def back():
                driver.back()
                painted()
                return {'bfcachePersisted': driver.execute_script('return window.wasPersisted ?? null')}
            measure('back', iteration, back)
            expected_urls[driver.current_window_handle] = driver.current_url
            driver.execute_script('scrollTo(0,0)')
            def scroll():
                return driver.execute_async_script('''
                  const done=arguments[0], intervals=[];
                  let last=performance.now(), frames=0;
                  function step(now) {
                    intervals.push(now-last); last=now; scrollBy(0,40);
                    if (++frames === 60) done({frameIntervalsMs:intervals, scrollY});
                    else requestAnimationFrame(step);
                  }
                  requestAnimationFrame(step);
                ''')
            measure('scroll', iteration, scroll)
        # Reports are deliberately outside latency measurements: collecting them
        # has overhead. Do not force GC, minimize memory, or trim the working set.
        for sample in range(args.samples):
            driver.set_context('chrome')
            report = assess_report(driver.execute_async_script(REPORT))
            report.update(sample=sample, timeUtc=datetime.now(timezone.utc).isoformat())
            result['allocator'].append(report)
            driver.set_context('content')
            if sample + 1 < args.samples:
                time.sleep(args.interval)
    except Exception as error:
        result['errors'].append(str(error))
        raise
    finally:
        result['summaryMs'] = {name: stats([r['ms'] for r in result['operations'] if r['operation'] == name and r['complete']])
                               for name in ('tab-switch', 'page-load', 'back', 'scroll')}
        frames = [v for r in result['operations'] if r['operation'] == 'scroll' and r['complete']
                  for v in r['detail']['frameIntervalsMs']]
        result['scrollFrameMs'] = stats(frames)
        result['dirtyBytes'] = stats([r['dirtyBytes'] for r in result['allocator'] if r['complete']])
        result['comparable'] = (not result['errors'] and len(result['operations']) == args.repeats * 4
                                and len(result['allocator']) == args.samples
                                and all(r['complete'] for r in result['allocator']))
        args.output.write_text(json.dumps(result, indent=2) + '\n')
        if driver:
            driver.quit()
        server.shutdown()
    print(args.output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--firefox', type=Path, required=True)
    parser.add_argument('--geckodriver', type=Path, required=True)
    parser.add_argument('--build', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--urls', type=Path, help='JSON array; same page order for every build')
    parser.add_argument('--repeats', type=int, default=10)
    parser.add_argument('--samples', type=int, default=24)
    parser.add_argument('--interval', type=float, default=5)
    parser.add_argument('--headless', action='store_true')
    parser.add_argument('--auto-unload', action='store_true')
    args = parser.parse_args()
    if args.repeats < 2 or args.samples < 2 or args.interval < 0:
        parser.error('at least 2 repeats/samples and a nonnegative interval required')
    if args.output.exists():
        parser.error('output already exists')
    run(args)
