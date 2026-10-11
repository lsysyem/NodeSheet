// 探针：验证 Slint ListView 横向滚动时，item(行) 的 width / TouchArea width / viewport-width 关系
slint::slint! {
import { ListView } from "std-widgets.slint";
export component Probe inherits Window {
    title: "lv-probe";
    in-out property <length> vw;           // ListView.viewport-width
    in-out property <length> iw: 0px;      // 第0行 item 的 width
    in-out property <length> taw: 0px;     // 第0行 TouchArea 的 width
    in-out property <length> cellsw: 0px;  // 第0行内容(cells)总宽

    lv := ListView {
        x: 0px; y: 0px;
        width: 400px; height: 300px;
        for d[i] in 1: Rectangle {
            height: 40px;
            min-width: cells.min-width;
            background: #e8e8e8;
            changed width => {
                root.iw = self.width;
                root.taw = ta.width;
                root.cellsw = cells.min-width;
            }
            ta := TouchArea { }
            cells := HorizontalLayout {
                Rectangle { min-width: 60px; max-width: 60px; }   // gutter
                for c[k] in 150: Rectangle {                     // 150 列，每列 100px
                    min-width: 100px; max-width: 100px;
                }
            }
        }
    }
    vw <=> lv.viewport-width;
}
}

fn main() {
    let app = Probe::new().unwrap();
    let t = slint::Timer::default();
    let weak = app.as_weak();
    t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(300), move || {
        if let Some(a) = weak.upgrade() {
            println!("RESULT viewport-width={} item-width={} toucharea-width={} cells-total-width={}",
                a.get_vw(), a.get_iw(), a.get_taw(), a.get_cellsw());
            println!("LV_WIDTH=400 (visible)");
            a.hide().ok();
            std::process::exit(0);
        }
    });
    app.show().unwrap();
    app.run().unwrap();
}
