#include "BattleManager.h"

#include "Async/Async.h"
#include "Camera/PlayerCameraManager.h"
#include "Components/DirectionalLightComponent.h"
#include "Components/InstancedStaticMeshComponent.h"
#include "Components/LineBatchComponent.h"
#include "Components/PostProcessComponent.h"
#include "Engine/StaticMesh.h"
#include "Engine/World.h"
#include "GameFramework/PlayerController.h"
#include "Materials/MaterialInstanceDynamic.h"
#include "UObject/ConstructorHelpers.h"
#include "VelaBattle.h"

namespace
{
	// How far the worker may run ahead of what is shown. The step that opens a
	// pulse also runs the AI commanders' look-ahead (up to about a second), so
	// keep a couple of seconds of steps in hand.
	constexpr size_t MaxAhead = 40;
	constexpr int32 MinStepsPerTask = 4;

	constexpr float BarrageRadius = 600.f;
	constexpr float ShieldBoostRange = 1500.f;
	constexpr float LayerChangeSeconds = 4.f;

	enum class EShape : uint8 { Cube, Sphere, Cylinder, Cone };

	// How each class is drawn: shape and size in distance units (length along
	// the heading, width, height). Lying shapes have their axis turned forward.
	// Structures are columns through both ship layers (Height < 0).
	struct FClassLook
	{
		EShape Shape;
		FVector Size;
		bool bLying;
	};
	const FClassLook ClassLooks[] = {
		{EShape::Cube, FVector(150, 55, 40), false},      // battleship
		{EShape::Cone, FVector(110, 50, 50), true},       // cruiser
		{EShape::Cone, FVector(60, 26, 26), true},        // destroyer
		{EShape::Cube, FVector(210, 75, 60), false},      // flagship
		{EShape::Cube, FVector(170, 95, 28), false},      // carrier
		{EShape::Cylinder, FVector(170, 36, 36), true},   // long-range gunship
		{EShape::Sphere, FVector(80, 80, 50), false},     // support
		{EShape::Cylinder, FVector(240, 240, -1), false}, // station
		{EShape::Cone, FVector(80, 80, -1), false},       // jump beacon
		{EShape::Cylinder, FVector(45, 45, -1), false},   // pylon
	};
	constexpr int32 NumClasses = UE_ARRAY_COUNT(ClassLooks);

	const FLinearColor SideColor[2] = {FLinearColor(0.10f, 0.35f, 1.0f), FLinearColor(1.0f, 0.18f, 0.12f)};
	const FLinearColor FlagshipColor[2] = {FLinearColor(0.55f, 0.8f, 1.0f), FLinearColor(1.0f, 0.62f, 0.3f)};
	const FLinearColor ShotColor[2] = {FLinearColor(0.3f, 0.85f, 1.0f), FLinearColor(1.0f, 0.55f, 0.15f)};

	bool IsStructure(uint8 Class)
	{
		return Class == BC_CLASS_STATION || Class == BC_CLASS_BEACON || Class == BC_CLASS_PYLON;
	}

	// Mesh-local scale and rotation for a class look (meshes are 100 units).
	void LookTransform(const FClassLook& Look, float Height, float Scale, FVector& OutScale, FQuat& OutTilt)
	{
		const FVector Size(Look.Size.X * Scale, Look.Size.Y * Scale, Look.Size.Z < 0 ? Height : Look.Size.Z * Scale);
		if (Look.bLying)
		{
			// Pitch -90 turns the mesh's +Z axis to +X: local (x, y, z) sizes
			// become world (z, y, x).
			OutScale = FVector(Size.Z, Size.Y, Size.X) / 100.f;
			OutTilt = FQuat(FRotator(-90.f, 0.f, 0.f));
		}
		else
		{
			OutScale = Size / 100.f;
			OutTilt = FQuat::Identity;
		}
	}

	// Cheap repeatable noise for placing rocks and debris.
	float Hash01(uint32 X)
	{
		X ^= X >> 16;
		X *= 0x7feb352dU;
		X ^= X >> 15;
		X *= 0x846ca68bU;
		X ^= X >> 16;
		return float(X & 0xFFFFFF) / float(0x1000000);
	}

	FString Utf8(const std::string& S)
	{
		return FString(UTF8_TO_TCHAR(S.c_str()));
	}
}

ABattleManager::ABattleManager()
{
	PrimaryActorTick.bCanEverTick = true;
	RootComponent = CreateDefaultSubobject<USceneComponent>(TEXT("Root"));

	static ConstructorHelpers::FObjectFinder<UStaticMesh> Cube(TEXT("/Engine/BasicShapes/Cube.Cube"));
	static ConstructorHelpers::FObjectFinder<UStaticMesh> Sphere(TEXT("/Engine/BasicShapes/Sphere.Sphere"));
	static ConstructorHelpers::FObjectFinder<UStaticMesh> Cylinder(TEXT("/Engine/BasicShapes/Cylinder.Cylinder"));
	static ConstructorHelpers::FObjectFinder<UStaticMesh> Cone(TEXT("/Engine/BasicShapes/Cone.Cone"));
	static ConstructorHelpers::FObjectFinder<UMaterialInterface> Material(TEXT("/Engine/BasicShapes/BasicShapeMaterial.BasicShapeMaterial"));
	CubeMesh = Cube.Object;
	SphereMesh = Sphere.Object;
	CylinderMesh = Cylinder.Object;
	ConeMesh = Cone.Object;
	BaseMaterial = Material.Object;

	StaticLines = CreateDefaultSubobject<ULineBatchComponent>(TEXT("StaticLines"));
	StaticLines->SetupAttachment(RootComponent);
	FrameLines = CreateDefaultSubobject<ULineBatchComponent>(TEXT("FrameLines"));
	FrameLines->SetupAttachment(RootComponent);
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

void ABattleManager::BeginPlay()
{
	Super::BeginPlay();
	SetActorLocationAndRotation(FVector::ZeroVector, FRotator::ZeroRotator);
	SetupLook();
	if (!VelaBattleCoreLoaded())
	{
		Phase = EBattlePhase::Failed;
		ErrorText = TEXT("找不到 battlecore.dll。請先執行 battle-ue/build_core.ps1，再重新開啟專案。");
		return;
	}
	StartBattle();
}

void ABattleManager::EndPlay(const EEndPlayReason::Type Reason)
{
	WaitForWorker();
	Sim.destroy(); // before the core DLL can be unloaded
	Super::EndPlay(Reason);
}

void ABattleManager::SetupLook()
{
	// Key light from above and a weak fill from below, both movable so no
	// light build is needed. Space stays black.
	auto MakeLight = [this](const FRotator& Rot, float Lux, const FLinearColor& Color, bool bShadows)
	{
		UDirectionalLightComponent* Light = NewObject<UDirectionalLightComponent>(this);
		Light->SetMobility(EComponentMobility::Movable);
		Light->SetupAttachment(RootComponent);
		Light->SetIntensity(Lux);
		Light->SetLightColor(Color);
		Light->SetCastShadows(bShadows);
		Light->SetRelativeRotation(Rot);
		Light->RegisterComponent();
		AddInstanceComponent(Light);
	};
	MakeLight(FRotator(-50.f, 35.f, 0.f), KeyLightLux, FLinearColor(1.0f, 0.96f, 0.9f), true);
	MakeLight(FRotator(40.f, -145.f, 0.f), FillLightLux, FLinearColor(0.6f, 0.7f, 1.0f), false);

	// Fixed exposure: auto exposure would brighten the black of space.
	UPostProcessComponent* Post = NewObject<UPostProcessComponent>(this);
	Post->bUnbound = true;
	Post->Settings.bOverride_AutoExposureMethod = true;
	Post->Settings.AutoExposureMethod = EAutoExposureMethod::AEM_Manual;
	Post->Settings.bOverride_AutoExposureBias = true;
	Post->Settings.AutoExposureBias = ExposureBias;
	Post->Settings.bOverride_AutoExposureApplyPhysicalCameraExposure = true;
	Post->Settings.AutoExposureApplyPhysicalCameraExposure = false;
	Post->SetupAttachment(RootComponent);
	Post->RegisterComponent();
	AddInstanceComponent(Post);

	for (int32 Side = 0; Side < 2; ++Side)
	{
		FighterSets.Add(MakeInstances(ConeMesh, FlagshipColor[Side], false));
		MissileSets.Add(MakeInstances(SphereMesh, ShotColor[Side], false));
	}
	FlashSets.Add(MakeInstances(SphereMesh, FLinearColor(1.0f, 0.45f, 0.08f), false)); // fire
	FlashSets.Add(MakeInstances(SphereMesh, FLinearColor(0.35f, 0.9f, 1.0f), false));  // shield
	FlashSets.Add(MakeInstances(SphereMesh, FLinearColor(0.9f, 0.95f, 1.0f), false));  // jump
}

UInstancedStaticMeshComponent* ABattleManager::MakeInstances(UStaticMesh* Mesh, const FLinearColor& Color, bool bShadows)
{
	UInstancedStaticMeshComponent* Set = NewObject<UInstancedStaticMeshComponent>(this);
	Set->SetStaticMesh(Mesh);
	UMaterialInstanceDynamic* Mat = UMaterialInstanceDynamic::Create(BaseMaterial, this);
	Mat->SetVectorParameterValue(TEXT("Color"), Color);
	Set->SetMaterial(0, Mat);
	Set->SetMobility(EComponentMobility::Movable);
	Set->SetCollisionEnabled(ECollisionEnabled::NoCollision);
	Set->SetCastShadow(bShadows);
	Set->SetupAttachment(RootComponent);
	Set->RegisterComponent();
	AddInstanceComponent(Set);
	return Set;
}

bool ABattleManager::StartBattle()
{
	WaitForWorker();
	Ahead.clear();
	Shots.Reset();
	Flashes.Reset();
	Rings.Reset();
	StrikeMarks.Reset();
	ReportText.Reset();
	if (!Sim.create(uint64(BattleSeed), uint32(BattleScale), bBlueAI, uint32(SimThreads)))
	{
		Phase = EBattlePhase::Failed;
		ErrorText = TEXT("模擬核心無法建立戰鬥。");
		return false;
	}
	GroupNames.Reset();
	for (const std::string& Name : Sim.group_names())
	{
		GroupNames.Add(Utf8(Name));
	}
	Sim.capture(Cur);
	Prev = Cur;
	Accum = 0.f;
	Alpha = 1.f;
	bWorkerParked = false;
	SelectedGroup = -1;
	for (int32 G = 0; G < int32(Cur.groups.size()); ++G)
	{
		if (Cur.groups[G].side == PlayerSide && !Cur.groups[G].fixed)
		{
			SelectedGroup = G;
			break;
		}
	}
	BuildShips();
	BuildTerrain(Sim.terrain());
	Phase = StopsAtPulse() ? EBattlePhase::Planning : EBattlePhase::Running;
	UE_LOG(LogVelaBattle, Log, TEXT("Battle %lld: %d ships, %d groups"), BattleSeed, int32(Sim.ships().size()), GroupNames.Num());
	return true;
}

void ABattleManager::BuildShips()
{
	for (UInstancedStaticMeshComponent* Set : ShipSets)
	{
		Set->DestroyComponent();
	}
	ShipSets.Reset();
	UStaticMesh* Meshes[] = {CubeMesh, SphereMesh, CylinderMesh, ConeMesh};
	for (int32 Class = 0; Class < NumClasses; ++Class)
	{
		for (int32 Side = 0; Side < 2; ++Side)
		{
			FLinearColor Color = Class == BC_CLASS_FLAGSHIP ? FlagshipColor[Side] : SideColor[Side];
			if (IsStructure(uint8(Class)))
			{
				Color = FMath::Lerp(Color, FLinearColor(0.6f, 0.6f, 0.6f), 0.45f);
			}
			ShipSets.Add(MakeInstances(Meshes[int32(ClassLooks[Class].Shape)], Color, true));
		}
	}
	const auto& Info = Sim.ships();
	const int32 Count = int32(Info.size());
	ShipSlot.SetNum(Count);
	ShipPos.SetNum(Count);
	ShipZ.SetNum(Count);
	ShipVisible.SetNum(Count);
	for (int32 I = 0; I < Count; ++I)
	{
		const int32 Class = FMath::Min<int32>(Info[I].ship_class, NumClasses - 1);
		const int32 SetIndex = Class * 2 + Info[I].side;
		const int32 Instance = ShipSets[SetIndex]->AddInstance(FTransform(FQuat::Identity, FVector::ZeroVector, FVector(0.001f)));
		ShipSlot[I] = FIntPoint(SetIndex, Instance);
		const vela::ShipState& S = Cur.ships[I];
		ShipZ[I] = IsStructure(Info[I].ship_class) ? 0.5f * (LowLayerZ + MainLayerZ) : LayerZ(vela::target_layer(S.layer));
		ShipPos[I] = SimToWorld(S.x, S.y, ShipZ[I]);
		ShipVisible[I] = S.state == BC_SHIP_ALIVE;
	}
}

void ABattleManager::BuildTerrain(const std::vector<uint8_t>& Cells)
{
	for (UInstancedStaticMeshComponent* Set : TerrainSets)
	{
		Set->DestroyComponent();
	}
	TerrainSets.Reset();

	const int32 Dim = int32(Sim.terrain_dim());
	const float Cell = float(Sim.terrain_cell());
	const float Half = Dim * 0.5f;
	auto CellCenter = [&](int32 CX, int32 CY, float Z)
	{
		return SimToWorld((CX - Half + 0.5f) * Cell, (CY - Half + 0.5f) * Cell, Z);
	};

	// Floor tiles: the map read from above.
	struct FTile
	{
		uint8 Kind;
		FLinearColor Color;
	};
	const FTile Tiles[] = {
		{BC_TERRAIN_ASTEROIDS, FLinearColor(0.22f, 0.15f, 0.08f)},
		{BC_TERRAIN_NEBULA, FLinearColor(0.32f, 0.08f, 0.42f)},
		{BC_TERRAIN_GRAVITY, FLinearColor(0.04f, 0.06f, 0.2f)},
		{BC_TERRAIN_DEBRIS, FLinearColor(0.16f, 0.16f, 0.16f)},
		{BC_TERRAIN_FIELD0, FLinearColor(0.04f, 0.1f, 0.28f)},
		{BC_TERRAIN_FIELD1, FLinearColor(0.28f, 0.05f, 0.04f)},
	};
	const FVector TileScale(Cell * 0.96f / 100.f, Cell * 0.96f / 100.f, 0.04f);
	for (const FTile& Tile : Tiles)
	{
		UInstancedStaticMeshComponent* Set = MakeInstances(CubeMesh, Tile.Color, false);
		TerrainSets.Add(Set);
		for (int32 CY = 0; CY < Dim; ++CY)
		{
			for (int32 CX = 0; CX < Dim; ++CX)
			{
				if (Cells[CY * Dim + CX] == Tile.Kind)
				{
					Set->AddInstance(FTransform(FQuat::Identity, CellCenter(CX, CY, FloorZ), TileScale));
				}
			}
		}
	}

	// Rocks through the low and main layers, wrecks on the low layer.
	UInstancedStaticMeshComponent* Rocks = MakeInstances(SphereMesh, FLinearColor(0.3f, 0.24f, 0.18f), true);
	UInstancedStaticMeshComponent* Wrecks = MakeInstances(CubeMesh, FLinearColor(0.25f, 0.25f, 0.27f), true);
	TerrainSets.Add(Rocks);
	TerrainSets.Add(Wrecks);
	for (int32 CY = 0; CY < Dim; ++CY)
	{
		for (int32 CX = 0; CX < Dim; ++CX)
		{
			const uint8 Kind = Cells[CY * Dim + CX];
			if (Kind != BC_TERRAIN_ASTEROIDS && Kind != BC_TERRAIN_DEBRIS)
			{
				continue;
			}
			const bool bRock = Kind == BC_TERRAIN_ASTEROIDS;
			const int32 Pieces = bRock ? 6 : 7;
			for (int32 K = 0; K < Pieces; ++K)
			{
				const uint32 Seed = uint32(CY * Dim + CX) * 977u + uint32(K) * 131u;
				const FVector Base = CellCenter(CX, CY, 0.f);
				const FVector Offset((Hash01(Seed) - 0.5f) * Cell * 0.9f, (Hash01(Seed + 1) - 0.5f) * Cell * 0.9f, 0.f);
				float Z, Size;
				if (bRock)
				{
					Z = FMath::Lerp(LowLayerZ - 150.f, MainLayerZ + 150.f, Hash01(Seed + 2));
					Size = FMath::Lerp(40.f, 150.f, Hash01(Seed + 3));
				}
				else
				{
					Z = LowLayerZ + (Hash01(Seed + 2) - 0.5f) * 160.f;
					Size = FMath::Lerp(15.f, 60.f, Hash01(Seed + 3));
				}
				const FVector Stretch(1.f, FMath::Lerp(0.6f, 1.f, Hash01(Seed + 4)), FMath::Lerp(0.5f, 0.9f, Hash01(Seed + 5)));
				const FQuat Rot(FRotator(Hash01(Seed + 6) * 360.f, Hash01(Seed + 7) * 360.f, Hash01(Seed + 8) * 360.f));
				(bRock ? Rocks : Wrecks)->AddInstance(FTransform(Rot, FVector(Base.X + Offset.X, Base.Y + Offset.Y, Z), Stretch * Size / 100.f));
			}
		}
	}

	// Planets: one sphere per connected patch of planet cells.
	UInstancedStaticMeshComponent* Planets = MakeInstances(SphereMesh, FLinearColor(0.18f, 0.32f, 0.45f), true);
	TerrainSets.Add(Planets);
	TArray<bool> Seen;
	Seen.Init(false, Dim * Dim);
	for (int32 Start = 0; Start < Dim * Dim; ++Start)
	{
		if (Seen[Start] || Cells[Start] != BC_TERRAIN_PLANET)
		{
			continue;
		}
		int32 MinX = Dim, MinY = Dim, MaxX = -1, MaxY = -1;
		TArray<int32> Stack = {Start};
		Seen[Start] = true;
		while (Stack.Num() > 0)
		{
			const int32 At = Stack.Pop();
			const int32 X = At % Dim, Y = At / Dim;
			MinX = FMath::Min(MinX, X);
			MaxX = FMath::Max(MaxX, X);
			MinY = FMath::Min(MinY, Y);
			MaxY = FMath::Max(MaxY, Y);
			const int32 Next[4][2] = {{X + 1, Y}, {X - 1, Y}, {X, Y + 1}, {X, Y - 1}};
			for (const auto& N : Next)
			{
				if (N[0] >= 0 && N[1] >= 0 && N[0] < Dim && N[1] < Dim)
				{
					const int32 NI = N[1] * Dim + N[0];
					if (!Seen[NI] && Cells[NI] == BC_TERRAIN_PLANET)
					{
						Seen[NI] = true;
						Stack.Push(NI);
					}
				}
			}
		}
		const float Diameter = FMath::Max(MaxX - MinX + 1, MaxY - MinY + 1) * Cell;
		const FVector Center = SimToWorld((0.5f * (MinX + MaxX + 1) - Half) * Cell, (0.5f * (MinY + MaxY + 1) - Half) * Cell, 0.5f * (LowLayerZ + MainLayerZ));
		Planets->AddInstance(FTransform(FQuat::Identity, Center, FVector(Diameter / 100.f)));
	}

	DrawStaticLines(Cells);
}

void ABattleManager::DrawStaticLines(const std::vector<uint8_t>& Cells)
{
	StaticLines->Flush();
	const int32 Dim = int32(Sim.terrain_dim());
	const float Cell = float(Sim.terrain_cell());
	const float Edge = Dim * 0.5f * Cell;

	// Layer sheets: a sparse grid on the low and main layers, a frame on the
	// fighter layer, so the three heights read from any angle.
	struct FSheet
	{
		float Z;
		FLinearColor Color;
		bool bGrid;
	};
	const FSheet Sheets[] = {
		{LowLayerZ, FLinearColor(0.05f, 0.16f, 0.16f), true},
		{MainLayerZ, FLinearColor(0.1f, 0.12f, 0.2f), true},
		{FighterLayerZ, FLinearColor(0.16f, 0.16f, 0.06f), false},
	};
	const float Step = 2000.f;
	for (const FSheet& Sheet : Sheets)
	{
		for (float V = -Edge; V <= Edge + 1.f; V += Step)
		{
			const bool bBorder = FMath::IsNearlyEqual(FMath::Abs(V), Edge, 1.f);
			if (!Sheet.bGrid && !bBorder)
			{
				continue;
			}
			StaticLines->DrawLine(SimToWorld(V, -Edge, Sheet.Z), SimToWorld(V, Edge, Sheet.Z), Sheet.Color, SDPG_World, 0.f, 0.f);
			StaticLines->DrawLine(SimToWorld(-Edge, V, Sheet.Z), SimToWorld(Edge, V, Sheet.Z), Sheet.Color, SDPG_World, 0.f, 0.f);
		}
	}

	// Outlines of nebulae (main and fighter layers) and asteroid fields (low
	// and main layers) at the heights where they act.
	auto Outline = [&](uint8 Kind, const FLinearColor& Color, float ZA, float ZB)
	{
		auto Is = [&](int32 X, int32 Y)
		{
			return X >= 0 && Y >= 0 && X < Dim && Y < Dim && Cells[Y * Dim + X] == Kind;
		};
		for (int32 CY = 0; CY < Dim; ++CY)
		{
			for (int32 CX = 0; CX < Dim; ++CX)
			{
				if (!Is(CX, CY))
				{
					continue;
				}
				const float X0 = (CX - Dim * 0.5f) * Cell, Y0 = (CY - Dim * 0.5f) * Cell;
				const float X1 = X0 + Cell, Y1 = Y0 + Cell;
				const float Edges[4][4] = {{X0, Y0, X1, Y0}, {X0, Y1, X1, Y1}, {X0, Y0, X0, Y1}, {X1, Y0, X1, Y1}};
				const bool Open[4] = {!Is(CX, CY - 1), !Is(CX, CY + 1), !Is(CX - 1, CY), !Is(CX + 1, CY)};
				for (int32 E = 0; E < 4; ++E)
				{
					if (!Open[E])
					{
						continue;
					}
					for (float Z : {ZA, ZB})
					{
						StaticLines->DrawLine(SimToWorld(Edges[E][0], Edges[E][1], Z), SimToWorld(Edges[E][2], Edges[E][3], Z), Color, SDPG_World, 0.f, 0.f);
					}
				}
			}
		}
	};
	Outline(BC_TERRAIN_NEBULA, FLinearColor(0.55f, 0.2f, 0.7f), MainLayerZ, FighterLayerZ);
	Outline(BC_TERRAIN_ASTEROIDS, FLinearColor(0.5f, 0.35f, 0.18f), LowLayerZ, MainLayerZ);
	Outline(BC_TERRAIN_GRAVITY, FLinearColor(0.15f, 0.2f, 0.55f), LowLayerZ, LowLayerZ);
}

// ---------------------------------------------------------------------------
// Simulation flow
// ---------------------------------------------------------------------------

bool ABattleManager::StopsAtPulse() const
{
	return !bBlueAI || bPauseAtPulseWhenWatching;
}

void ABattleManager::WaitForWorker()
{
	if (Pending.IsValid())
	{
		Pending.Wait();
		Pending = TFuture<std::vector<vela::Snapshot>>();
	}
}

void ABattleManager::HarvestWorker()
{
	if (Pending.IsValid() && Pending.IsReady())
	{
		std::vector<vela::Snapshot> Got = Pending.Consume();
		Pending = TFuture<std::vector<vela::Snapshot>>();
		const bool bStopAtPulse = StopsAtPulse();
		for (vela::Snapshot& S : Got)
		{
			const bool bStop = S.over() || (bStopAtPulse && S.at_pulse());
			Ahead.push_back(std::move(S));
			if (bStop)
			{
				bWorkerParked = true;
			}
		}
	}
}

void ABattleManager::PumpWorker()
{
	HarvestWorker();
	if (Pending.IsValid())
	{
		return;
	}
	if (Phase == EBattlePhase::Running && !bWorkerParked && Ahead.size() < MaxAhead)
	{
		const bool bStopAtPulse = StopsAtPulse();
		// Enough steps per task to keep up with fast playback at 30 fps.
		const int32 Steps = FMath::Max(MinStepsPerTask, FMath::CeilToInt32(SimSpeed * BC_TICK_HZ / 30.f));
		Pending = Async(EAsyncExecution::ThreadPool, [this, bStopAtPulse, Steps]()
		{
			std::vector<vela::Snapshot> Out;
			Out.reserve(Steps);
			for (int32 K = 0; K < Steps; ++K)
			{
				Out.emplace_back();
				const bool bRunning = Sim.step(Out.back());
				if (!bRunning || (bStopAtPulse && Out.back().at_pulse()))
				{
					break;
				}
			}
			return Out;
		});
	}
}

void ABattleManager::Advance(float DeltaSeconds)
{
	if (Phase != EBattlePhase::Running)
	{
		Alpha = 1.f;
		return;
	}
	Accum += DeltaSeconds * BC_TICK_HZ * SimSpeed;
	while (Accum >= 1.f)
	{
		if (Ahead.empty())
		{
			Accum = 1.f; // the worker is behind (AI thinking); hold the last step
			break;
		}
		std::swap(Prev, Cur);
		Cur = std::move(Ahead.front());
		Ahead.pop_front();
		Accum -= 1.f;
		OnStep(Prev, Cur);
		if (Cur.over())
		{
			WaitForWorker();
			Phase = EBattlePhase::Over;
			ReportText = Utf8(Sim.report());
			Accum = 1.f;
			break;
		}
		// Stop only at the pulse the worker parked on. If blue was handed back
		// from the AI mid-pulse, the worker may already be past this one.
		if (Cur.at_pulse() && StopsAtPulse() && Ahead.empty() && !Pending.IsValid())
		{
			Phase = EBattlePhase::Planning;
			Accum = 1.f;
			break;
		}
	}
	Alpha = FMath::Clamp(Accum, 0.f, 1.f);
}

void ABattleManager::Go()
{
	if (Phase == EBattlePhase::Planning)
	{
		Prev = Cur;
		Accum = 0.f;
		bWorkerParked = false;
		Phase = EBattlePhase::Running;
	}
}

void ABattleManager::Restart(int64 NewSeed)
{
	BattleSeed = NewSeed;
	if (VelaBattleCoreLoaded())
	{
		StartBattle();
	}
}

void ABattleManager::SetBlueAI(bool bAI)
{
	if (Phase == EBattlePhase::Failed || Phase == EBattlePhase::Over)
	{
		return;
	}
	// Keep the steps the worker already made; they are past the point of no return.
	if (Pending.IsValid())
	{
		Pending.Wait();
		HarvestWorker();
	}
	bBlueAI = bAI;
	Sim.set_ai(0, bAI);
	ShowMessage(bAI ? TEXT("藍方交給 AI 指揮（下一個脈衝起生效）") : TEXT("藍方由你指揮（下一個脈衝起生效）"));
	if (bAI && Phase == EBattlePhase::Planning && !StopsAtPulse())
	{
		Go();
	}
}

int32 ABattleManager::IssueOrder(int32 Group, int32 Verb, int32 A, int32 B)
{
	if (Phase != EBattlePhase::Planning)
	{
		ShowMessage(TEXT("只能在脈衝暫停時下令"));
		return BC_ERR_BAD_COMMAND;
	}
	if (Group < 0)
	{
		ShowMessage(TEXT("先選一個藍方戰鬥群"));
		return BC_ERR_BAD_COMMAND;
	}
	const int32 Result = Sim.issue(PlayerSide, uint32(Group), uint32(Verb), A, B);
	if (Result != BC_OK)
	{
		ShowMessage(UTF8_TO_TCHAR(vela::command_error(Result)));
		return Result;
	}
	Sim.capture(Cur);
	Prev = Cur;
	return Result;
}

void ABattleManager::ShowMessage(const FString& Text)
{
	MessageText = Text;
	MessageTime = GetWorld() ? GetWorld()->GetTimeSeconds() : 0.0;
}

double ABattleManager::MessageAge() const
{
	return GetWorld() ? double(GetWorld()->GetTimeSeconds()) - MessageTime : 1000.0;
}

// Effects that start on a step: explosions, shots, flashes.
void ABattleManager::OnStep(const vela::Snapshot& Before, const vela::Snapshot& After)
{
	const auto& Info = Sim.ships();
	auto ShipAt = [&](uint32 Ship) -> FVector
	{
		const vela::ShipState& S = After.ships[Ship];
		return SimToWorld(S.x, S.y, ShipZ.IsValidIndex(int32(Ship)) ? ShipZ[Ship] : MainLayerZ);
	};
	const uint32 Count = uint32(After.ships.size());

	for (uint32 I = 0; I < Count && I < uint32(Before.ships.size()); ++I)
	{
		const uint8 Was = Before.ships[I].state, Now = After.ships[I].state;
		if (Was == BC_SHIP_ALIVE && Now == BC_SHIP_DEAD)
		{
			AddFlash(ShipAt(I), ShipLength(I) * 1.6f, 0, 1.4f);
			AddFlash(ShipAt(I), ShipLength(I) * 0.8f, 2, 0.4f);
		}
		else if (Was == BC_SHIP_ALIVE && Now == BC_SHIP_ESCAPED)
		{
			AddFlash(ShipAt(I), ShipLength(I) * 1.2f, 2, 0.7f);
		}
	}

	for (const BcEvent& E : After.events)
	{
		switch (E.kind)
		{
		case BC_EV_FIRE:
			if (E.a < Count && E.b < Count && Shots.Num() < 6000)
			{
				FShot Shot;
				Shot.From = E.a;
				Shot.To = E.b;
				Shot.Side = Info[E.a].side;
				Shot.DamageType = E.d;
				Shot.bHit = (E.c & 1) != 0;
				Shot.bSalvo = (E.c & 2) != 0;
				Shot.Age = 0.f;
				Shot.Life = E.d == 0 ? 0.18f : 0.45f;
				Shot.MissOffset = Shot.bHit ? FVector::ZeroVector : FVector(FMath::FRandRange(-1.f, 1.f), FMath::FRandRange(-1.f, 1.f), FMath::FRandRange(-0.4f, 0.4f)) * ShipLength(E.b) * 1.5f;
				Shots.Add(Shot);
			}
			break;
		case BC_EV_MISSILE_HIT:
			if (E.b < Count)
			{
				AddFlash(ShipAt(E.b), ShipLength(E.b) * 0.7f, 0, 0.5f);
			}
			break;
		case BC_EV_MISSILE_INTERCEPTED:
			for (const BcMissile& M : Before.missiles)
			{
				if (M.id == E.b)
				{
					AddFlash(SimToWorld(float(M.x), float(M.y), MainLayerZ), 30.f, 0, 0.3f);
					break;
				}
			}
			break;
		case BC_EV_FIGHTER_DOWN:
			for (const BcWing& W : After.wings)
			{
				if (W.id == E.a)
				{
					AddFlash(SimToWorld(float(W.x), float(W.y), FighterLayerZ) + FVector(FMath::FRandRange(-80.f, 80.f), FMath::FRandRange(-80.f, 80.f), 0.f), 25.f, 0, 0.4f);
					break;
				}
			}
			break;
		case BC_EV_BARRAGE_MARKED:
			StrikeMarks.Add(E.a, FVector2D(float(int32(E.b)), float(E.c)));
			break;
		case BC_EV_BARRAGE_HIT:
			if (const FVector2D* Mark = StrikeMarks.Find(E.a))
			{
				for (float Z : {LowLayerZ, MainLayerZ})
				{
					AddFlash(SimToWorld(float(Mark->X), float(Mark->Y), Z), BarrageRadius * 1.6f, 0, 1.0f);
				}
				StrikeMarks.Remove(E.a);
			}
			break;
		case BC_EV_SIGNATURE_FIRED:
			if (E.a < uint32(After.groups.size()))
			{
				const BcGroup& G = After.groups[E.a];
				AddRing(SimToWorld(float(G.cx), float(G.cy), LayerZ(G.layer)), 900.f, FLinearColor(1.f, 0.9f, 0.4f), 0.8f);
			}
			break;
		case BC_EV_SHIELD_BOOST:
			if (E.a < Count)
			{
				AddRing(ShipAt(E.a), ShieldBoostRange, FLinearColor(0.35f, 0.9f, 1.f), 1.2f);
			}
			break;
		case BC_EV_JUMP_IN:
			for (uint32 I = 0; I < Count; ++I)
			{
				if (Info[I].group == E.a && After.ships[I].state == BC_SHIP_ALIVE)
				{
					AddFlash(ShipAt(I), ShipLength(I) * 1.5f, 2, 0.6f);
				}
			}
			break;
		default:
			break;
		}
	}

	if (!After.terrain.empty())
	{
		BuildTerrain(After.terrain);
	}
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

void ABattleManager::Tick(float DeltaSeconds)
{
	Super::Tick(DeltaSeconds);
	Clock += DeltaSeconds;
	if (APlayerController* PC = GetWorld()->GetFirstPlayerController())
	{
		if (PC->PlayerCameraManager)
		{
			CameraPos = PC->PlayerCameraManager->GetCameraLocation();
		}
	}
	if (Phase == EBattlePhase::Failed)
	{
		return;
	}
	PumpWorker();
	Advance(DeltaSeconds);
	PumpWorker();

	UpdateShips(DeltaSeconds);
	UpdateFighters();
	UpdateMissiles();
	UpdateEffects(DeltaSeconds);
	DrawFrameLines();
}

FVector ABattleManager::SimToWorld(float X, float Y, float Z) const
{
	// Sim x runs right and y down, as in the 2D viewer; Unreal X is forward.
	return FVector(-Y, X, Z);
}

FVector2D ABattleManager::WorldToSim(const FVector& World) const
{
	return FVector2D(float(World.Y), float(-World.X));
}

float ABattleManager::LayerZ(uint8 Layer) const
{
	return (Layer & 0x7f) == BC_LAYER_LOW ? LowLayerZ : MainLayerZ;
}

const FString& ABattleManager::GroupName(int32 Group) const
{
	static const FString None;
	return GroupNames.IsValidIndex(Group) ? GroupNames[Group] : None;
}

FVector ABattleManager::GroupWorld(int32 Group) const
{
	if (Group < 0 || Group >= int32(Cur.groups.size()))
	{
		return FVector::ZeroVector;
	}
	const BcGroup& G = Cur.groups[Group];
	FVector2D C(float(G.cx), float(G.cy));
	if (Group < int32(Prev.groups.size()))
	{
		C = FMath::Lerp(FVector2D(float(Prev.groups[Group].cx), float(Prev.groups[Group].cy)), C, Alpha);
	}
	// Average drawn height of the group's ships, so climbing groups move smoothly.
	float Z = 0.f;
	int32 N = 0;
	const auto& Info = Sim.ships();
	for (int32 I = 0; I < ShipZ.Num(); ++I)
	{
		if (Info[I].group == Group && ShipShown(I))
		{
			Z += ShipZ[I];
			++N;
		}
	}
	return SimToWorld(float(C.X), float(C.Y), N > 0 ? Z / N : LayerZ(G.layer));
}

float ABattleManager::ShipLength(int32 Ship) const
{
	const auto& Info = Sim.ships();
	if (Ship < 0 || Ship >= int32(Info.size()))
	{
		return 50.f;
	}
	const FClassLook& Look = ClassLooks[FMath::Min<int32>(Info[Ship].ship_class, NumClasses - 1)];
	return FMath::Max(Look.Size.X, Look.Size.Y) * ShipScale;
}

void ABattleManager::SetPool(UInstancedStaticMeshComponent* Pool, const TArray<FTransform>& Transforms)
{
	const FTransform Hidden(FQuat::Identity, FVector(0.f, 0.f, -100000.f), FVector(0.001f));
	while (Pool->GetInstanceCount() < Transforms.Num())
	{
		Pool->AddInstance(Hidden);
	}
	TArray<FTransform> All = Transforms;
	while (All.Num() < Pool->GetInstanceCount())
	{
		All.Add(Hidden);
	}
	if (All.Num() > 0)
	{
		Pool->BatchUpdateInstancesTransforms(0, All, false, true, true);
	}
}

void ABattleManager::UpdateShips(float DeltaSeconds)
{
	const auto& Info = Sim.ships();
	const int32 Count = int32(Info.size());
	if (Count == 0 || int32(Cur.ships.size()) != Count)
	{
		return;
	}
	const float ClimbRate = FMath::Abs(MainLayerZ - LowLayerZ) / LayerChangeSeconds * (Phase == EBattlePhase::Running ? SimSpeed : 0.f);
	TArray<TArray<FTransform>> PerSet;
	PerSet.SetNum(ShipSets.Num());
	for (int32 Set = 0; Set < ShipSets.Num(); ++Set)
	{
		PerSet[Set].SetNum(ShipSets[Set]->GetInstanceCount());
	}
	const bool bHavePrev = int32(Prev.ships.size()) == Count;
	for (int32 I = 0; I < Count; ++I)
	{
		const vela::ShipState& S = Cur.ships[I];
		const vela::ShipState& P = bHavePrev ? Prev.ships[I] : S;
		const uint8 Class = Info[I].ship_class;
		const bool bStructure = IsStructure(Class);
		const FClassLook& Look = ClassLooks[FMath::Min<int32>(Class, NumClasses - 1)];

		// Height: ease toward the layer the ship is in or moving to.
		if (!bStructure)
		{
			const float Target = LayerZ(vela::target_layer(S.layer));
			ShipZ[I] = FMath::FInterpConstantTo(ShipZ[I], Target, DeltaSeconds, ClimbRate);
		}

		const float X = FMath::Lerp(P.x, S.x, Alpha);
		const float Y = FMath::Lerp(P.y, S.y, Alpha);
		ShipPos[I] = SimToWorld(X, Y, ShipZ[I]);
		ShipVisible[I] = S.state == BC_SHIP_ALIVE;

		FVector LocalScale;
		FQuat Tilt;
		LookTransform(Look, FMath::Abs(MainLayerZ - LowLayerZ) + 300.f, ShipScale, LocalScale, Tilt);
		const float HeadingDeg = FMath::RadiansToDegrees(vela::heading_radians(vela::lerp_heading(P.heading, S.heading, Alpha)));
		const FQuat Rot = FQuat(FRotator(0.f, HeadingDeg + 90.f, 0.f)) * Tilt;
		if (!ShipVisible[I])
		{
			LocalScale = FVector(0.001f);
		}
		const FIntPoint Slot = ShipSlot[I];
		PerSet[Slot.X][Slot.Y] = FTransform(Rot, ShipPos[I], LocalScale);

		// Burning hulls throw sparks.
		if (ShipVisible[I] && S.fx.hull_pct < 35 && Phase == EBattlePhase::Running && FMath::FRand() < DeltaSeconds * 3.f)
		{
			const float L = ShipLength(I);
			AddFlash(ShipPos[I] + FVector(FMath::FRandRange(-0.4f, 0.4f) * L, FMath::FRandRange(-0.4f, 0.4f) * L, 0.f), L * 0.25f, 0, 0.5f);
		}
	}
	for (int32 Set = 0; Set < ShipSets.Num(); ++Set)
	{
		if (PerSet[Set].Num() > 0)
		{
			ShipSets[Set]->BatchUpdateInstancesTransforms(0, PerSet[Set], false, true, true);
		}
	}
}

void ABattleManager::UpdateFighters()
{
	TMap<uint32, FVector2D> Before;
	for (const BcWing& W : Prev.wings)
	{
		Before.Add(W.id, FVector2D(float(W.x), float(W.y)));
	}
	TArray<FTransform> PerSide[2];
	const FVector FighterScale(0.12f * ShipScale, 0.12f * ShipScale, 0.3f * ShipScale);
	const FQuat Tilt(FRotator(-90.f, 0.f, 0.f));
	for (const BcWing& W : Cur.wings)
	{
		FVector2D C(float(W.x), float(W.y));
		if (const FVector2D* B = Before.Find(W.id))
		{
			C = FMath::Lerp(*B, C, Alpha);
		}
		const FVector Center = SimToWorld(float(C.X), float(C.Y), FighterLayerZ);
		// Fighters circle their wing's position; the sim tracks the wing only.
		for (int32 K = 0; K < W.count; ++K)
		{
			const float Phase0 = Hash01(W.id * 31u + uint32(K)) * 2.f * UE_PI;
			const float Radius = 50.f + 70.f * Hash01(W.id * 17u + uint32(K) * 7u);
			const float Angle = Phase0 + Clock * (1.2f + 0.6f * Hash01(uint32(K) + 99u));
			const FVector Offset(FMath::Cos(Angle) * Radius, FMath::Sin(Angle) * Radius, FMath::Sin(Angle * 2.f + Phase0) * 40.f);
			const float Yaw = FMath::RadiansToDegrees(Angle) + 90.f;
			PerSide[W.side & 1].Add(FTransform(FQuat(FRotator(0.f, Yaw, 0.f)) * Tilt, Center + Offset, FighterScale));
		}
	}
	SetPool(FighterSets[0], PerSide[0]);
	SetPool(FighterSets[1], PerSide[1]);
}

void ABattleManager::UpdateMissiles()
{
	TMap<uint32, FVector2D> Before;
	for (const BcMissile& M : Prev.missiles)
	{
		Before.Add(M.id, FVector2D(float(M.x), float(M.y)));
	}
	TArray<FTransform> PerSide[2];
	for (const BcMissile& M : Cur.missiles)
	{
		FVector2D C(float(M.x), float(M.y));
		if (const FVector2D* B = Before.Find(M.id))
		{
			C = FMath::Lerp(*B, C, Alpha);
		}
		// Missiles fly at the target's height.
		float Z = MainLayerZ;
		if (M.target < uint32(ShipZ.Num()))
		{
			Z = ShipZ[M.target];
		}
		const float Size = (M.kind == 1 ? 0.5f : 0.22f) * ShipScale;
		PerSide[M.side & 1].Add(FTransform(FQuat::Identity, SimToWorld(float(C.X), float(C.Y), Z), FVector(Size)));
	}
	SetPool(MissileSets[0], PerSide[0]);
	SetPool(MissileSets[1], PerSide[1]);
}

void ABattleManager::AddFlash(const FVector& Pos, float Size, uint8 Kind, float Life)
{
	if (Flashes.Num() < 3000)
	{
		Flashes.Add({Pos, Size, 0.f, Life, Kind});
	}
}

void ABattleManager::AddRing(const FVector& Center, float Radius, const FLinearColor& Color, float Life)
{
	Rings.Add({Center, Radius, 0.f, Life, Color});
}

void ABattleManager::UpdateEffects(float DeltaSeconds)
{
	const float Dt = Phase == EBattlePhase::Running ? DeltaSeconds * SimSpeed : 0.f;
	TArray<FTransform> PerKind[3];
	for (int32 K = Flashes.Num() - 1; K >= 0; --K)
	{
		FFlash& F = Flashes[K];
		F.Age += Dt;
		if (F.Age >= F.Life)
		{
			Flashes.RemoveAtSwap(K);
			continue;
		}
		const float T = F.Age / F.Life;
		const float Grow = 0.3f + 0.7f * FMath::Sqrt(T);
		const float Fade = T < 0.6f ? 1.f : 1.f - (T - 0.6f) / 0.4f;
		PerKind[FMath::Min<int32>(F.Kind, 2)].Add(FTransform(FQuat::Identity, F.Pos, FVector(F.Size * Grow * Fade / 100.f)));
	}
	for (int32 Kind = 0; Kind < 3; ++Kind)
	{
		SetPool(FlashSets[Kind], PerKind[Kind]);
	}
	for (int32 K = Shots.Num() - 1; K >= 0; --K)
	{
		Shots[K].Age += Dt;
		if (Shots[K].Age >= Shots[K].Life)
		{
			FShot& S = Shots[K];
			// Impact as the shot lands: shield shimmer or hull sparks.
			if (S.bHit && S.To < uint32(ShipPos.Num()) && ShipShown(int32(S.To)))
			{
				const bool bShield = S.To < uint32(Cur.ships.size()) && Cur.ships[S.To].fx.shield_pct > 0;
				const FVector Dir = (ShipPos[S.From] - ShipPos[S.To]).GetSafeNormal();
				const float L = ShipLength(int32(S.To));
				AddFlash(ShipPos[S.To] + Dir * L * 0.5f, L * (S.bSalvo ? 0.9f : 0.35f), bShield ? 1 : 0, 0.25f);
			}
			Shots.RemoveAtSwap(K);
		}
	}
	for (int32 K = Rings.Num() - 1; K >= 0; --K)
	{
		Rings[K].Age += Dt;
		if (Rings[K].Age >= Rings[K].Life)
		{
			Rings.RemoveAtSwap(K);
		}
	}
}

float ABattleManager::LineWidth(const FVector& At, float Pixels) const
{
	// Roughly constant on-screen width at any zoom (60 degree view).
	return Pixels * float(FVector::Dist(CameraPos, At)) * 0.0012f;
}

void ABattleManager::DrawCircle(const FVector& Center, float Radius, const FLinearColor& Color, float Width, int32 Segments)
{
	FVector Last = Center + FVector(Radius, 0.f, 0.f);
	for (int32 K = 1; K <= Segments; ++K)
	{
		const float A = 2.f * UE_PI * K / Segments;
		const FVector Next = Center + FVector(FMath::Cos(A) * Radius, FMath::Sin(A) * Radius, 0.f);
		FrameLines->DrawLine(Last, Next, Color, SDPG_World, Width, 0.f);
		Last = Next;
	}
}

void ABattleManager::DrawFrameLines()
{
	FrameLines->Flush();
	const auto& Info = Sim.ships();
	const int32 Count = ShipPos.Num();

	// Shots: beams flash whole, kinetic rounds and missiles travel as tracers.
	for (const FShot& S : Shots)
	{
		if (S.From >= uint32(Count) || S.To >= uint32(Count))
		{
			continue;
		}
		const FVector A = ShipPos[S.From];
		const FVector B = ShipPos[S.To] + S.MissOffset;
		const float T = S.Age / S.Life;
		FLinearColor Color = ShotColor[S.Side & 1];
		const float Px = S.bSalvo ? 3.f : 1.2f;
		if (S.DamageType == 0)
		{
			Color *= 1.f - T;
			FrameLines->DrawLine(A, B, Color, SDPG_World, LineWidth(A, Px), 0.f);
		}
		else
		{
			const FVector Head = FMath::Lerp(A, B, FMath::Min(1.f, T * 1.1f));
			const FVector Tail = FMath::Lerp(A, B, FMath::Max(0.f, T * 1.1f - 0.18f));
			FrameLines->DrawLine(Tail, Head, Color, SDPG_World, LineWidth(Head, Px * 1.5f), 0.f);
		}
	}

	// Height cues: a drop line from each ship to the floor.
	if (bDropLines)
	{
		for (int32 I = 0; I < Count; ++I)
		{
			if (ShipShown(I))
			{
				const FLinearColor C = SideColor[Info[I].side & 1] * 0.25f;
				FrameLines->DrawLine(ShipPos[I], FVector(ShipPos[I].X, ShipPos[I].Y, FloorZ), C, SDPG_World, 0.f, 0.f);
			}
		}
	}

	// Groups: where they head, what they fight, signature moves charging.
	const int32 Groups = int32(Cur.groups.size());
	for (int32 G = 0; G < Groups; ++G)
	{
		const BcGroup& Gr = Cur.groups[G];
		if (Gr.alive == 0 || Gr.fixed || Gr.status == BC_GROUP_ESCAPED || Gr.status == BC_GROUP_DESTROYED)
		{
			continue;
		}
		const FVector Center = GroupWorld(G);
		const bool bSelected = G == SelectedGroup;
		if (Gr.side == PlayerSide || bSelected)
		{
			const FVector Goal = SimToWorld(float(Gr.goal_x), float(Gr.goal_y), Center.Z);
			FrameLines->DrawLine(Center, Goal, FLinearColor(0.2f, 0.6f, 0.25f) * (bSelected ? 1.f : 0.5f), SDPG_World, bSelected ? LineWidth(Center, 1.5f) : 0.f, 0.f);
		}
		if (bSelected && Gr.target_group >= 0 && Gr.target_group < Groups)
		{
			const FVector Target = GroupWorld(Gr.target_group);
			FrameLines->DrawLine(Center, Target, FLinearColor(0.9f, 0.25f, 0.2f), SDPG_World, LineWidth(Center, 1.5f), 0.f);
		}
		if (Gr.signature < 0)
		{
			const float Pulse = 0.5f + 0.5f * FMath::Sin(Clock * 12.f);
			DrawCircle(Center, 700.f + 150.f * Pulse, FLinearColor(1.f, 0.85f, 0.3f), LineWidth(Center, 2.f), 40);
		}
	}

	// Selection: a ring under every ship of the selected group.
	if (SelectedGroup >= 0)
	{
		for (int32 I = 0; I < Count; ++I)
		{
			if (Info[I].group == SelectedGroup && ShipShown(I))
			{
				DrawCircle(ShipPos[I], ShipLength(I) * 0.75f, FLinearColor(1.f, 0.95f, 0.3f), LineWidth(ShipPos[I], 1.2f), 16);
			}
		}
	}

	// Marked barrages: the landing zone on both ship layers, closing in.
	for (const BcStrike& St : Cur.strikes)
	{
		const float Left = FMath::Clamp(float(St.ticks_left) / float(5 * BC_TICK_HZ), 0.f, 1.f);
		for (float Z : {LowLayerZ, MainLayerZ})
		{
			const FVector C = SimToWorld(float(St.x), float(St.y), Z);
			DrawCircle(C, BarrageRadius, ShotColor[St.side & 1], LineWidth(C, 2.f));
			DrawCircle(C, BarrageRadius * Left, ShotColor[St.side & 1] * 0.6f, LineWidth(C, 1.f));
		}
	}

	for (const FRing& R : Rings)
	{
		const float T = R.Age / R.Life;
		DrawCircle(R.Center, R.Radius * (0.4f + 0.6f * T), R.Color * (1.f - T), LineWidth(R.Center, 2.f));
	}

	// Fighter wings diving on a ship.
	for (const BcWing& W : Cur.wings)
	{
		if (W.state == BC_WING_OUT && W.target < uint32(Count) && ShipShown(int32(W.target)) && FMath::Fmod(Clock * 3.f + W.id * 0.37f, 1.f) < 0.3f)
		{
			const FVector From = SimToWorld(float(W.x), float(W.y), FighterLayerZ);
			FrameLines->DrawLine(From, ShipPos[W.target], ShotColor[W.side & 1] * 0.7f, SDPG_World, 0.f, 0.f);
		}
	}
}
