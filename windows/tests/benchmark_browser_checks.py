#!/usr/bin/env python3
"""Real Firefox checks for scrolling and rejection of non-scroll workloads."""
import argparse
from pathlib import Path
from urllib.parse import quote

from selenium import webdriver
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.firefox.service import Service

from benchmark_browser import SCROLL
from benchmark_metrics import validate_scroll


def run(binary, geckodriver):
    options = Options()
    options.binary_location = str(binary.resolve())
    options.add_argument('-headless')
    with webdriver.Firefox(options=options, service=Service(str(geckodriver.resolve()))) as driver:
        driver.set_script_timeout(10)
        for name, style, valid in [('long', 'body{height:10000px}', True),
                                   ('short', '', False),
                                   ('blocked', 'body{height:10000px}html{overflow:hidden}', False)]:
            driver.get('data:text/html,' + quote('<!doctype html><style>' + style + '</style><p>Scroll test</p>'))
            result = driver.execute_async_script(SCROLL)
            if valid:
                validate_scroll(result)
            else:
                try:
                    validate_scroll(result)
                except ValueError:
                    pass
                else:
                    raise AssertionError(f'{name} was accepted: {result}')
            print('PASS', name)


if __name__ == '__main__':
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--firefox', type=Path, default=root / 'linux/runtime/firefox/firefox')
    parser.add_argument('--geckodriver', type=Path, default=root / '.tools/geckodriver')
    args = parser.parse_args()
    run(args.firefox, args.geckodriver)
