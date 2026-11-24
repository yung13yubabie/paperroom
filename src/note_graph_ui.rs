use super::*;
use crate::store::note_links::Scope;
use crate::store::note_links::LinkIndex;

type Neighbor = (i64, String, String);

pub(in crate::window) struct Graph {
    pub(super) title: String,
    focus: i64,
    incoming: Vec<Neighbor>,
    outgoing: Vec<Neighbor>,
    offset: usize,
    overview: Option<Overview>,
}

#[derive(Clone, Copy)]
struct Layout {
    area: RECT,
    gap: i32,
    width: i32,
    step: i32,
    top: i32,
    visible: usize,
}

impl Layout {
    fn new(app: &App, area: RECT) -> Self {
        let gap = app.scaled(12);
        let width = ((area.right - area.left - gap * 4) / 3).max(1);
        let top = area.top + app.scaled(44);
        let step = app.scaled(60).max(1);
        let visible = ((area.bottom - top).max(0) / step) as usize;
        Self { area, gap, width, step, top, visible }
    }
    fn card(self, column: i32, index: usize) -> RECT {
        let left = self.area.left + self.gap + column * (self.width + self.gap);
        let top = self.top + index as i32 * self.step;
        RECT {left, top, right:left+self.width, bottom:top+self.step-self.gap}
    }
    fn focus(self) -> RECT { self.card(1, self.visible.saturating_sub(1)/2) }
}

impl Graph {
    fn last_offset(&self, visible: usize) -> usize {
        self.outgoing.len().max(self.incoming.len()).saturating_sub(visible.max(1))
    }
    fn visible_cards(&self, layout: Layout) -> impl Iterator<Item=(&Neighbor, RECT)> {
        let start = self.offset.min(self.last_offset(layout.visible));
        [(0, &self.incoming), (2, &self.outgoing)].into_iter().flat_map(move |(col, list)| {
            list.iter().skip(start).take(layout.visible).enumerate()
                .map(move |(i, node)| (node, layout.card(col,i)))
        })
    }
}

unsafe fn graph_card(app: &App, dc:HDC, mut rect:RECT, title:&str, detail:&str, center:bool) {
    unsafe {
        fill(dc,&rect, if center {MINT} else {PAPER});
        rect.left += app.scaled(5);
        rect.right -= app.scaled(5);
        let mut second = rect;
        second.top = (rect.top+rect.bottom)/2;
        rect.bottom = second.top;
        draw_text(dc,app.fonts[4],INK,title,&mut rect,DT_VCENTER|DT_SINGLELINE|DT_END_ELLIPSIS|DT_NOPREFIX);
        draw_text(dc,app.fonts[4],SOFT,detail,&mut second,DT_VCENTER|DT_SINGLELINE|DT_PATH_ELLIPSIS|DT_NOPREFIX);
    }
}

impl App {
    pub(in crate::window) unsafe fn open_note_graph(&mut self, id:i64) -> Result<()> {
        let note = self.notebook.note(id)?.filter(|n|n.row.trash_root.is_none()).ok_or("這篇筆記已移至垃圾桶或不存在。")?;
        let (outgoing,incoming) = self.notebook.graph_neighbors(&note)?;
        unsafe {self.activate_workspace(WorkspaceView::Graph(Graph {
            title:note.row.title, focus:id, incoming,outgoing,offset:0,overview:None,
        }));}
        Ok(())
    }
    pub(super) unsafe fn paint_graph(&self, dc:HDC, area:RECT, graph:&Graph) {
        unsafe {
            if let Some(view)=&graph.overview {self.paint_overview(dc,area,view,graph.focus);return;}
            let layout = Layout::new(self,area);
            for (col,title) in [format!("引用此頁（{}）",graph.incoming.len()), "目前筆記".into(),
                                 format!("此頁連到（{}）",graph.outgoing.len())].into_iter().enumerate() {
                let mut r=layout.card(col as i32,0);
                r.top=area.top; r.bottom=layout.top;
                draw_text(dc,self.fonts[4],TEAL,&title,&mut r,DT_SINGLELINE|DT_VCENTER|DT_END_ELLIPSIS);
            }
            let focus=layout.focus();
            for (_,r) in graph.visible_cards(layout) {
                let (from,to) = if r.left < focus.left {(r,focus)} else {(focus,r)};
                let y1=(from.top+from.bottom)/2;
                let y2=(to.top+to.bottom)/2;
                MoveToEx(dc,from.right,y1,null_mut()); LineTo(dc,to.left,y2);
                MoveToEx(dc,to.left-4,y2-3,null_mut()); LineTo(dc,to.left,y2); LineTo(dc,to.left-4,y2+3);
            }
            for (node,r) in graph.visible_cards(layout) {graph_card(self,dc,r,&node.1,&node.2,false);}
            graph_card(self,dc,focus,&graph.title,"開啟目前筆記",true);
            if graph.incoming.is_empty() && graph.outgoing.is_empty() {
                let mut hint=area; hint.top=focus.bottom+self.scaled(6);
                draw_text(dc,self.fonts[4],SOFT,"尚無可解析的連結。開啟目前筆記，從「連結」插入筆記連結。",&mut hint,DT_CENTER|DT_WORDBREAK);
            }
        }
    }
    pub(super) unsafe fn graph_input(&mut self,msg:u32,wp:WPARAM,lp:LPARAM)->Result<()> {
        if matches!(&self.extras.workspace,Some(WorkspaceView::Graph(g)) if g.overview.is_some()) {
            return unsafe {self.overview_input(msg,wp,lp)};
        }
        let layout = Layout::new(self,unsafe { self.board_area() });
        let Some(WorkspaceView::Graph(graph))=&mut self.extras.workspace else {return Ok(());};
        let start=graph.offset.min(graph.last_offset(layout.visible));
        if msg==WM_MOUSEWHEEL {
            let delta=(wp>>16) as u16 as i16;
            graph.offset=if delta>0 {start.saturating_sub(1)} else {start.saturating_add(1).min(graph.last_offset(layout.visible))};
            unsafe {InvalidateRect(self.hwnd,null(),1);}
        } else if msg==WM_LBUTTONUP {
            let x=lp as u16 as i16 as i32;
            let y=(lp>>16) as u16 as i16 as i32;
            let hit=|r:RECT| x>=r.left && x<r.right && y>=r.top && y<r.bottom;
            let target=if hit(layout.focus()) {Some(graph.focus)} else {
                graph.visible_cards(layout).find(|(_,r)|hit(*r)).map(|(node,_)|node.0)
            };
            if let Some(id)=target {unsafe {self.open_anchored_note(id,None)?;}}
        }
        Ok(())
    }
    pub(super) unsafe fn graph_menu(&mut self)->Result<()> {
        unsafe {
            let Some(WorkspaceView::Graph(graph))=&self.extras.workspace else {return Ok(());};
            if graph.overview.is_some() {return self.overview_menu();}
            let focus=graph.focus;
            let layout=Layout::new(self,self.board_area());
            let start=graph.offset.min(graph.last_offset(layout.visible));
            let mut entries=vec![(1,"開啟目前筆記".to_string()),(2,"重新整理關聯圖".to_string())];
            if start>0 {entries.push((3,"上一頁".into()));}
            if start<graph.last_offset(layout.visible) {entries.push((4,"下一頁".into()));}
            let mut ids=Vec::new();
            for (node,r) in graph.visible_cards(layout) {
                let side=if r.left<layout.focus().left {"引用此頁"} else {"此頁連到"};
                entries.push((ids.len()+5,format!("{side}　{}　{}",node.1,node.2)));
                ids.push(node.0);
            }
            let all_id=ids.len()+5;
            entries.push((all_id,"整個工作區關聯圖".into()));
            let vault=self.notebook.graph_vault(focus)?;
            if vault.is_some() { entries.push((all_id+1,"目前 Vault 關聯圖".into())); }
            let menu=entries.iter().map(|(id,s)|(*id,s.as_str())).collect::<Vec<_>>();
            match popup(self.hwnd,self.widgets.links,None,&menu) {
                0 => (),
                1 => self.open_anchored_note(focus,None)?,
                2 => self.open_note_graph(focus)?,
                step @ (3|4) => {
                    if let Some(WorkspaceView::Graph(graph))=&mut self.extras.workspace {
                        let count=layout.visible.max(1);
                        graph.offset=if step==3 {start.saturating_sub(count)} else {(start+count).min(graph.last_offset(count))};
                        InvalidateRect(self.hwnd,null(),1);
                    }
                }
                index if index==all_id => self.open_graph_overview(focus,Scope::Workspace)?,
                index if index==all_id+1 && vault.is_some() => self.open_graph_overview(focus,Scope::Vault(vault.unwrap()))?,
                index => if let Some(id)=ids.get(index-5) {self.open_anchored_note(*id,None)?;},
            }
            Ok(())
        }
    }
}

struct Overview {
    pub scope: Scope,
    index: LinkIndex,
    nodes: Vec<i64>,
    page: usize,
    route: bool,
}

const PAGE: usize = 9;

impl Overview {
    fn new(scope:Scope,index:LinkIndex,focus:i64)->Self {
        let nodes:Vec<_>=index.nodes.keys().copied().collect();
        let page=nodes.iter().position(|id|*id==focus).unwrap_or(0)/PAGE;
        Self {scope,index,nodes,page,route:false}
    }

    fn shown(&self)->&[i64] {
        let start=(self.page*PAGE).min(self.nodes.len());
        &self.nodes[start..(start+PAGE).min(self.nodes.len())]
    }

    fn rect(&self,app:&App,area:RECT,ordinal:usize)->RECT {
        let top=area.top+app.scaled(54);
        let w=(area.right-area.left).max(3);
        let h=(area.bottom-top).max(3);
        let col=ordinal as i32%3;
        let row=ordinal as i32/3;
        let cx=area.left+(col*2+1)*w/6;
        let cy=top+(row*2+1)*h/6;
        let half=(w/6-app.scaled(8)).max(1);
        let height=app.scaled(23).min((h/6).max(1));
        RECT {left:cx-half,right:cx+half,top:cy-height,bottom:cy+height}
    }
}

impl App {
    pub(super) unsafe fn open_graph_overview(&mut self,focus:i64,scope:Scope)->Result<()> {
        let index=self.notebook.link_index(scope,None)?;
        let title=match scope {Scope::Workspace=>"整個工作區".to_owned(),Scope::Vault(id)=>{
            let vault=self.notebook.meta(id)?.ok_or("找不到此 Vault。")?;
            format!("Vault · {}",vault.title)
        }};
        unsafe {self.activate_workspace(WorkspaceView::Graph(Graph {
            title,focus,incoming:Vec::new(),outgoing:Vec::new(),offset:0,
            overview:Some(Overview::new(scope,index,focus)),
        }));}
        Ok(())
    }

    pub(super) unsafe fn paint_overview(&self,dc:HDC,area:RECT,view:&Overview,focus:i64) {
        unsafe {
            let shown=view.shown();
            let mut header=area;
            header.left+=self.scaled(8);
            header.bottom=area.top+self.scaled(50);
            let heading=if view.route {format!("有向路徑：{} 步",view.nodes.len().saturating_sub(1))}
                else {format!("{} 篇筆記，{} 組有向連結",view.index.nodes.len(),view.index.edges.len())};
            let pages=view.nodes.len().div_ceil(PAGE).max(1);
            draw_text(dc,self.fonts[4],INK,&format!("{heading}　│　第 {} / {pages} 頁　│　顯示同頁連線",view.page+1),
                &mut header,DT_VCENTER|DT_SINGLELINE|DT_END_ELLIPSIS);
            for (i,&from) in shown.iter().enumerate() {
                for (j,&to) in shown.iter().enumerate() {
                    if !view.index.edges.contains(&(from,to)) || (view.route && j!=i+1) {continue;}
                    let a=view.rect(self,area,i); let b=view.rect(self,area,j);
                    let (ax,ay)=((a.left+a.right)/2,(a.top+a.bottom)/2);
                    let (bx,by)=((b.left+b.right)/2,(b.top+b.bottom)/2);
                    if i==j {
                        let x=ax+self.scaled(8); let y=a.top-self.scaled(5);
                        MoveToEx(dc,ax,a.top,null_mut());LineTo(dc,ax,y);LineTo(dc,x,y);LineTo(dc,x,a.top);
                        continue;
                    }
                    let (dx,dy)=((bx-ax) as f64,(by-ay) as f64);
                    let f=(f64::from((b.right-b.left)/2)/dx.abs()).min(f64::from((b.bottom-b.top)/2)/dy.abs());
                    let x=f64::from(bx)-dx*f; let y=f64::from(by)-dy*f;
                    let length=dx.hypot(dy);
                    let (ux,uy)=(dx/length,dy/length);
                    MoveToEx(dc,ax,ay,null_mut());LineTo(dc,x as i32,y as i32);
                    let size=f64::from(self.scaled(7));
                    for side in [-0.45,0.45] {
                        MoveToEx(dc,x as i32,y as i32,null_mut());
                        LineTo(dc,(x-size*(ux+side*uy)) as i32,(y-size*(uy-side*ux)) as i32);
                    }
                }
            }
            for (i,id) in shown.iter().enumerate() {
                let node=&view.index.nodes[id];
                graph_card(self,dc,view.rect(self,area,i),&node.row.title,&node.detail(),*id==focus);
            }
        }
    }

    pub(super) unsafe fn overview_input(&mut self,msg:u32,wp:WPARAM,lp:LPARAM)->Result<()> {
        let area=unsafe {self.board_area()};
        let Some(WorkspaceView::Graph(graph))=&self.extras.workspace else {return Ok(());};
        let Some(view)=&graph.overview else {return Ok(());};
        if msg==WM_LBUTTONUP {
            let x=lp as u16 as i16 as i32;
            let y=(lp>>16) as u16 as i16 as i32;
            let target=view.shown().iter().enumerate().find(|(i,_)|{
                let r=view.rect(self,area,*i);x>=r.left&&x<r.right&&y>=r.top&&y<r.bottom
            }).map(|(_,id)|*id);
            if let Some(id)=target {unsafe {self.open_anchored_note(id,None)?;}}
        } else if msg==WM_MOUSEWHEEL {
            let back=(wp>>16) as u16 as i16>0;
            unsafe {self.graph_page(back);}
        }
        Ok(())
    }

    unsafe fn graph_page(&mut self,back:bool) {
        if let Some(WorkspaceView::Graph(graph))=&mut self.extras.workspace
            && let Some(view)=&mut graph.overview {
            let last=view.nodes.len().saturating_sub(1)/PAGE;
            view.page=if back {view.page.saturating_sub(1)} else {(view.page+1).min(last)};
            unsafe {InvalidateRect(self.hwnd,null(),1);}
        }
    }

    pub(super) unsafe fn overview_menu(&mut self)->Result<()> {
        unsafe {
            let Some(WorkspaceView::Graph(graph))=&self.extras.workspace else {return Ok(());};
            let focus=graph.focus;
            let Some(view)=&graph.overview else {return Ok(());};
            let scope=view.scope;
            let mut entries=vec![(1,"重新整理全圖".to_owned()),(2,"回到目前筆記的關聯圖".into()),
                (3,"尋找有向路徑…".into()),(4,"切換至整個工作區".into())];
            if self.notebook.graph_vault(focus)?.is_some() {entries.push((5,"切換至目前 Vault".into()));}
            if view.page>0 {entries.push((6,"上一頁".into()));}
            if (view.page+1)*PAGE<view.nodes.len() {entries.push((7,"下一頁".into()));}
            for (i,id) in view.shown().iter().enumerate() {
                let node=&view.index.nodes[id];entries.push((8+i,format!("開啟　{}　{}",node.row.title,node.detail())));
            }
            let menu=entries.iter().map(|(id,s)|(*id,s.as_str())).collect::<Vec<_>>();
            match popup(self.hwnd,self.widgets.links,None,&menu) {
                0=>(),
                1=>self.open_graph_overview(focus,scope)?,
                2=>self.open_note_graph(focus)?,
                3=>{
                    let Some(pair)=crate::dialog::prompt(self.hwnd,"尋找有向路徑","輸入起點與終點筆記編號，以空格分隔（可含 #）",&format!("#{focus} "),96)? else {return Ok(());};
                    let ids=pair.split_whitespace().map(|part|part.trim_start_matches('#').parse::<i64>())
                        .collect::<std::result::Result<Vec<_>,_>>().map_err(|_|"請輸入兩個筆記編號。")?;
                    let [from,to]=ids.as_slice() else {return Err("請輸入起點與終點兩個筆記編號。".into());};
                    if !view.index.nodes.contains_key(from)||!view.index.nodes.contains_key(to) {return Err("起點或終點不在目前範圍中。".into());}
                    let route=view.index.shortest_path(*from,*to).ok_or("這兩篇筆記之間沒有依此方向可到達的路徑。")?;
                    if let Some(WorkspaceView::Graph(graph))=&mut self.extras.workspace && let Some(view)=&mut graph.overview {
                        view.nodes=route;view.page=0;view.route=true;
                    }
                    InvalidateRect(self.hwnd,null(),1);
                    label(self.widgets.status,"已顯示最短有向路徑。點選節點可開啟筆記，重新整理可回全圖。");
                }
                4=>self.open_graph_overview(focus,Scope::Workspace)?,
                5=>{
                    let root=self.notebook.graph_vault(focus)?.ok_or("這篇筆記不在匯入的 Vault 中。")?;
                    self.open_graph_overview(focus,Scope::Vault(root))?;
                }
                6=>self.graph_page(true),
                7=>self.graph_page(false),
                n=>if let Some(id)=view.shown().get(n-8) {self.open_anchored_note(*id,None)?;},
            }
            Ok(())
        }
    }
}
