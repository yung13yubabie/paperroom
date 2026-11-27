function Test-NotionReverse32([string]$Database) {
    $ids=@'
import json,sqlite3,sys,pathlib
with sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+"?mode=ro",uri=True) as db:
    rows=db.execute("""select c.id,n.id,m.status from collections c join collection_notes m on m.collection_id=c.id
      join notes n on m.note_id=n.id where c.name='導演集合' and n.title='場景資料列'""").fetchall()
    assert len(rows)==1
    print(json.dumps(rows[0], ensure_ascii=True))
'@ | python -B - $Database
    if ($LASTEXITCODE -ne 0 -or !$ids) {throw 'Cannot resolve unique fixture row'}
    $collection,$row,$status=($ids | ConvertFrom-Json)
    $label="導演集合　[#$collection]　｜ $status　｜ 場景資料列　[#$row]"
    foreach ($action in 1,2) {
        if ($action -eq 2) {
            Native-Menu 120 60001 '具名關聯的來源與目標…'
            Native-Menu 120 2 '此筆記指向的具名關聯…' -MenuOpen
            Choose-CollectionRow 'Notion Markdown 來源　同名圖卡'
        }
        $null=Wait-For 'reverse navigation starts on exact source body' {
            [PaperroomHighlightGui]::Read((Control $window 121)).Contains('乙：待核對的正文')
        }
        Native-Menu 120 60001 '具名關聯的來源與目標…'
        Native-Menu 120 4 '查看 Notion 來源頁的集合資料列…' -MenuOpen
        $options=@(Collection-MenuLabels)
        if ($options.Count -ne 1 -or $options[0] -cne $label) {throw "Unexpected reverse rows: $options"}
        Choose-CollectionRow $label
        if ($action -eq 1) {
            Native-Menu 120 1 '開啟資料列筆記' -MenuOpen
            $null=Wait-For 'canonical CSV row opened' {
                [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '場景資料列' -and
                [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '已開啟 Notion 來源頁的資料列。'
            }
        } else {
            Native-Menu 120 2 '在集合看板定位' -MenuOpen
            $null=Wait-For 'exact row focused in imported collection' {
                [PaperroomHighlightGui]::Read((Control $window 115)).Contains('導演集合') -and
                [PaperroomHighlightGui]::Read((Control $window 116)) -ceq "已定位集合「導演集合」的資料列 #$row。"
            }
            Open-SortNamedView 1 '保留的來源檢視'
            Native-Menu 120 3 '開啟資料列…'
            $savedRows=@(Collection-MenuLabels)
            if ($savedRows.Count -ne 1 -or $savedRows[0] -cne "$status　場景資料列  ($row)") {throw "Saved named view lost its member: $savedRows"}
            Choose-CollectionRow $savedRows[0]
            $null=Wait-For 'saved named view still opens its exact CSV member' {
                [PaperroomHighlightGui]::Read((Control $window 103)) -ceq '場景資料列'
            }
            # Opening a member already returns to the ordinary note view.
            $null=Wait-For 'returned from positioned collection' {
                [PaperroomHighlightGui]::Read((Control $window 119)) -ceq '卡片白板'
            }
        }
    }
    Select-Note '工作資料'
    Native-Menu 120 2 '前往　精確甲　｜ Pages' -CaptionPrefix
    $null=Wait-For 'unbound same-title source body' {
        [PaperroomHighlightGui]::Read((Control $window 121)).Contains('甲：另一份內容')
    }
    Native-Menu 120 60001 '具名關聯的來源與目標…'
    Native-Menu 120 4 '查看 Notion 來源頁的集合資料列…' -MenuOpen
    $null=Wait-For 'unbound page cannot borrow a same-title source relation' {
        [PaperroomHighlightGui]::Read((Control $window 116)) -ceq '這個 Notion 來源頁沒有可開啟的集合資料列。'
    }
}
