"""テストの誤りの文は英語（元の文）で比べる（OS の言語によらない。lensbench-core の i18n.rs、XCT-20）。"""

import os

os.environ.setdefault("LENSBENCH_LANG", "en")
