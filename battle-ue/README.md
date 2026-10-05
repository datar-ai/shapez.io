# battle-ue：戰鬥核心的 3D 畫面（Unreal Engine 5.8）

設計文件〈戰鬥核心技術設計〉第 3 步。規則、AI 和結果全部在 `../battle-core`（Rust）裡算，這個 UE 專案只負責顯示和輸入。

專案不需要任何美術資源：艦船、戰機、飛彈、爆炸都用引擎內建的基本形狀，光束、曳光彈、選取圈和地形邊界用線條。之後換成正式模型和 Niagara 特效時，只要換 `ABattleManager` 裡的對應段落，流程不用動。

## 在 Windows 上建置

需要：

- Unreal Engine 5.8（Epic Games Launcher 安裝即可）。
- Visual Studio 2022，勾選「使用 C++ 的遊戲開發」和 UE 安裝程式要求的 Windows SDK。
- Rust（https://rustup.rs ，預設的 `x86_64-pc-windows-msvc` 工具鏈）。

步驟：

1. 編譯模擬核心並把 DLL 放到專案裡。在 repo 根目錄開 PowerShell：

   ```powershell
   powershell -ExecutionPolicy Bypass -File battle-ue\build_core.ps1
   ```

   會產生 `battle-ue\Source\ThirdParty\BattleCore\lib\Win64\battlecore.dll`、`battlecore.dll.lib`，並複製一份 DLL 到 `battle-ue\Binaries\Win64\`。改過 `battle-core` 的 Rust 程式碼後要重跑一次。

2. 在檔案總管對 `battle-ue\VelaBattle.uproject` 按右鍵，選「Generate Visual Studio project files」。

3. 打開 `VelaBattle.sln`，組態選 `Development Editor`、平台 `Win64`，建置 `VelaBattle`。

4. 雙擊 `VelaBattle.uproject` 開啟編輯器，按「Play」。

   - 預設地圖是引擎內建的空地圖 `/Engine/Maps/Entry`。遊戲模式會自己生出戰鬥、燈光、鏡頭和 HUD，所以任何地圖都能玩。
   - 想調參數（種子、規模、層高、艦船放大倍數、燈光、曝光）：開一個新的空關卡，從「Place Actors」拖一個 `BattleManager` 進去，在細節面板改，存檔後把它設成預設地圖。

如果畫面左上角出現「找不到 battlecore.dll」，表示第 1 步沒跑或 DLL 沒複製到；重跑第 1 步再重開編輯器。

打包成遊戲時，`BattleCore.Build.cs` 會把 DLL 複製到執行檔旁邊，不用另外處理。

## 操作

| 類別 | 按鍵 |
|---|---|
| 鏡頭 | WASD 或方向鍵平移（Shift 加速），滾輪縮放，Q／E 旋轉，中鍵拖曳環繞，PageUp／PageDown 調俯角，F 對準選中的戰鬥群，Home 回到預設視角 |
| 選取 | 左鍵點艦船、戰鬥群標籤或左側列表；Tab 換下一個藍方戰鬥群 |
| 指令（脈衝暫停時） | 右鍵敵方戰鬥群＝攻擊（按住 Ctrl 左側擊、Alt 右側擊）；右鍵空地＝推進到該點（取戰鬥群所在高度層的平面）；H 固守、R 撤離、T 招牌技、L 換層、C 投入預備隊、1／2／3 準則（砲列／鐵砧／鐵鎚）、5／6／7／8 集火引擎／武器／護盾產生器／艦橋 |
| 戰鬥 | 空白鍵開打下一個 60 秒脈衝；P 把藍方交給 AI（觀戰）或收回；N 換下一個種子開新戰鬥；＋／－ 播放速度；B 開關高度線 |

流程跟網頁版一樣：每個脈衝開始時暫停，你有 3 點指令點，下完令按空白鍵，60 秒後再停。紅方由內建 AI 指揮。

## 畫面上看到的東西

- **三個高度層**：下層（軌道與地形層）、主戰層、戰機層，預設高度 −900、0、+800。下層和主戰層各有一張淡色網格，戰機層只有外框。每艘艦船有一條垂直線拉到最底下的地形地板，從斜角也看得出高度（B 關掉）。換層時艦船在 4 秒內平滑升降。
- **艦船**：依軍種用不同形狀，藍紅兩色，旗艦較亮。戰艦、旗艦是長方塊，航艦是扁平方塊，巡洋艦、驅逐艦是指向前方的錐體，長程砲艦是長圓柱，支援艦是球。空間站、信標、能量場塔是貫穿兩層的柱子（建築同時在兩層）。艦船預設畫成碰撞大小的 2 倍，遠看才認得出來。
- **地形**：最底下一層地板用色塊標出地形種類（小行星帶棕、星雲紫、重力井深藍、殘骸灰、能量場藍或紅）。小行星帶在下層到主戰層之間散布岩塊，殘骸在下層，星球是一顆大球。星雲在主戰層和戰機層畫紫色外框，小行星帶在下層和主戰層畫棕色外框。殘骸和能量場在戰鬥中變化時會重畫。
- **戰鬥**：光束是一閃即逝的直線，動能砲是飛行的曳光段，齊射較粗。命中時有護盾閃光（青）或船體火花（橘），船體低於 35% 會持續冒火花。飛彈、魚雷是小球，魚雷較大。戰機在戰機層繞著中隊位置盤旋，俯衝攻擊時有細線連到目標。擊沉有爆炸，撤離有白色閃光。
- **戰鬥群**：頭上有名稱和士氣條。己方畫出前往目標點的綠線；選中的戰鬥群畫出攻擊目標的紅線，每艘艦船腳下有黃圈。招牌技充能時群外圍有閃爍的黃圈。長程砲艦的落點在兩個艦船層各畫一個圈，內圈隨倒數縮小，上面標秒數。
- **HUD**：上方是種子、脈衝、雙方指令點和狀態；左側是雙方戰鬥群列表（艦數、狀態、準則、所在層、招牌技、戰機、集火部位、士氣）；下方是按鍵說明；結束時中央顯示戰報。

## 程式結構

| 檔案 | 內容 |
|---|---|
| `Source/ThirdParty/BattleCore/BattleCore.Build.cs` | 把 Rust 核心當成外部模組：標頭直接用 `../battle-core/include/battlecore.h`，連結 `battlecore.dll.lib`，DLL 延遲載入並隨遊戲打包 |
| `Source/VelaBattle/Private/VelaBattleModule.cpp` | 遊戲啟動時從固定位置載入 `battlecore.dll`；找不到就在畫面上顯示錯誤，不會當掉 |
| `Public/BattleBridge.h`、`Private/BattleBridge.cpp` | 純 C++17 的核心包裝（不依賴 UE）：建立戰鬥、每步取出快照（艦船、飛彈、戰機、戰鬥群、落點、事件、地形變化）、下指令、錯誤訊息、戰報 |
| `Public/BattleManager.h`、`Private/BattleManager.cpp` | 在背景執行緒跑模擬，遊戲執行緒以每秒 20 步播放並在兩步之間內插；用 Instanced Static Mesh 畫艦船、戰機、飛彈、爆炸和地形，用線條畫射擊與標記 |
| `BattlePlayerController` | 鏡頭、選取、下指令（直接讀按鍵狀態，不需要 Input Action 資產） |
| `BattleHUD` | Canvas 畫的 HUD |
| `BattleGameMode`、`BattleCameraPawn` | 把以上組起來 |
| `Tests/` | 不用 UE 就能跑的橋接層測試 |

**執行緒**：模擬每個脈衝開始的那一步，AI 指揮官會往前模擬幾種走法（最多約 1 秒）。所以模擬放在背景執行緒，最多領先畫面 40 步（2 秒），畫面不會卡。玩家只在脈衝暫停時下令，這時背景執行緒一定是閒置的，核心不會被兩個執行緒同時存取。

**座標**：模擬平面的 1 單位 = UE 的 1 單位（公分）。模擬的 x 向右、y 向下（跟網頁版俯視圖一樣），對到 UE 是 Y＝x、X＝−y，所以鏡頭預設方向看到的左右和網頁版相同：藍方在左、紅方在右。

## 驗證狀態

在這個雲端環境（Linux，沒有 UE）能驗證的部分：

- `battle-core` 的 30 項 Rust 測試全過，包括新增的兩項：C 結構的大小和欄位位置跟標頭一致，以及新的型別化 C API 在整場戰鬥中跟艦船陣列一致。
- `Tests/run.sh` 用 g++（`-Wall -Wextra -Wshadow -Werror`）編譯橋接層並對真正的核心跑完：整場自動戰鬥（88 艘，3 個脈衝，45 艘被擊沉）的最終雜湊跟直接呼叫 C API 完全相同；玩家下令、指令點用完被拒、換層 4 秒後落到下層、60 秒後回到暫停都正確。標頭裡每個結構的大小和欄位位置用 `static_assert` 固定。
- 網頁版用的 wasm 也照常編得過。

**UE 端的 C++ 沒有實際編譯過**，因為這裡沒有 UE。第一次在你的電腦上建置時，如果有編譯錯誤，把錯誤訊息貼回 thread 就好；也可以讓我透過 Remote Control 直接在你的電腦上建置和除錯。

## 已知限制與下一步

完整的 3A 規劃（渲染、美術資產、特效、鏡頭、介面、音效、效能預算、工具鏈、里程碑）見文件〈[戰鬥畫面 3A 製作規劃](https://claude.ai/code/artifact/cfc2fe9c-2532-4e94-9b37-e8b069dcdf17)〉；這個專案是其中的 M0。


- 特效是佔位：爆炸和閃光是會長大縮小的實心球（受光照，不發光），光束是線。下一步換成 Niagara（光束、曳光、爆炸、護盾波紋），並給材質加 per-instance custom data，讓艦船依受損程度變色。
- HUD 字型用引擎的 Roboto，靠引擎的後備字型顯示中文。若中文變成方塊，匯入 Noto Sans TC 之類的字型、設成 Runtime 字型，把路徑填進 `Config/DefaultGame.ini` 的 `FontPath`。
- 艦船等用的是引擎的 `BasicShapeMaterial`。編輯器裡沒問題；打包成遊戲時，若這個材質沒勾「Used with Instanced Static Meshes」，實例會顯示成預設材質。M1 換成專案自己的材質（同時加 per-instance 顏色）就解決。
- 目前只會開示範戰鬥（`bc_create_demo`）。之後要從戰略層帶入編成時，在 C API 加一個用資料建立戰鬥的入口。
- 大規模（`BattleScale` 調高到幾千艘）時，每步複製一份快照的成本和線條數量還沒量過。
