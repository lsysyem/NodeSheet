"""生成测试用的 ods。
python testdata/make_ods.py                    → testdata/big200k.ods（20 万行，bench 用）
python testdata/make_ods.py testdata/small.ods 6 → 小的（回归测试用，照 make_small.py 的用法）"""
import sys

import pandas as pd

path = sys.argv[1] if len(sys.argv) > 1 else "testdata/big200k.ods"
n = int(sys.argv[2]) if len(sys.argv) > 2 else 200_000
df = pd.DataFrame({
    "id": range(1, n + 1),
    "name": [f"客户{i % 10000:05d}" for i in range(n)],
    "amount": [round(1000 + (i % 977) * 3.7, 2) for i in range(n)],
    "qty": [i % 500 for i in range(n)],
    "city": [["北京", "上海", "广州", "深圳", "杭州"][i % 5] for i in range(n)],
    "active": [i % 2 == 0 for i in range(n)],
    "score": [round((i % 1000) / 7, 4) for i in range(n)],
    "remark": [f"备注{i % 1000}" if i % 3 else "" for i in range(n)],
    "date": pd.date_range("2023-01-01", periods=n, freq="D"),
})
df.to_excel(path, index=False, engine="odf")
print(f"written {path}")
