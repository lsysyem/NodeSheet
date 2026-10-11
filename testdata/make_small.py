# 生成几个小测试文件（单元测试用，生成后提交进仓库）：
#   dates.xlsx      各种日期 / 时间 / 时长格式（内置编号、自定义、中文格式）
#   gbk.csv         GBK 编码（中文 Windows 上 Excel“另存为 CSV”的默认编码）
#   semicolon.csv   分号分隔
# 用法：python testdata/make_small.py（要 xlsxwriter）
import datetime as dt
import os

import xlsxwriter

here = os.path.dirname(os.path.abspath(__file__))

wb = xlsxwriter.Workbook(os.path.join(here, "dates.xlsx"))
ws = wb.add_worksheet("日期")
formats = [
    ("日期", "yyyy-mm-dd"),
    ("日期时间", "yyyy-mm-dd hh:mm:ss"),
    ("时间", "hh:mm:ss"),
    ("中文日期", 'yyyy"年"m"月"d"日"'),
    ("短日期", 14),
    ("时长", "[h]:mm"),
    ("数字", "0.00"),
]
rows = [
    [dt.date(2023, 7, 15), dt.datetime(2023, 7, 15, 12, 30), dt.time(8, 5), dt.date(2023, 7, 15), dt.date(2023, 7, 15), 1.5, 1234.5],
    [dt.date(1999, 12, 31), dt.datetime(2000, 1, 1, 0, 0, 1), dt.time(23, 59, 59), dt.date(2024, 2, 29), dt.date(1900, 3, 1), 0.25, 0.1],
]
for c, (name, _) in enumerate(formats):
    ws.write(0, c, name)
for r, row in enumerate(rows, start=1):
    for c, value in enumerate(row):
        fmt = wb.add_format({"num_format": formats[c][1]})
        if isinstance(value, (dt.date, dt.time)):
            ws.write_datetime(r, c, value, fmt)
        else:
            ws.write_number(r, c, value, fmt)
wb.close()

with open(os.path.join(here, "gbk.csv"), "w", encoding="gbk", newline="") as f:
    f.write('姓名,城市,金额\r\n张三,北京,12.5\r\n李四,上海,8\r\n"赵六,七",深圳,3\r\n')

with open(os.path.join(here, "semicolon.csv"), "w", encoding="utf-8", newline="") as f:
    f.write("名称;数量;单价\n苹果;3;1.5\n香蕉;5;2.25\n")
