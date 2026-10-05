// Runs one battle-core battle and draws it in 3D.
//
// The core steps on a worker thread a little ahead of what is shown; the game
// thread plays the steps back at 20 per second (times SimSpeed) and blends
// between the last two so motion is smooth at any frame rate. Orders are only
// given at pulse boundaries, when the worker is idle, so the core is never
// touched from two threads at once.
//
// Everything is drawn from engine basic shapes and debug-style lines so the
// project needs no content: ships are instanced static meshes per class and
// side, fighters/missiles/explosions are pooled instances, beams and tracers
// are lines. Swap in real meshes and Niagara later without touching the flow.
#pragma once

#include "CoreMinimal.h"
#include "Async/Future.h"
#include "GameFramework/Actor.h"

#include <deque>
#include <vector>

#include "BattleBridge.h"

#include "BattleManager.generated.h"

class UInstancedStaticMeshComponent;
class ULineBatchComponent;
class UMaterialInterface;
class UStaticMesh;

UENUM()
enum class EBattlePhase : uint8
{
	// Waiting at a pulse boundary for the player's orders.
	Planning,
	Running,
	Over,
	// The core DLL is missing or the battle could not be created.
	Failed,
};

UCLASS()
class VELABATTLE_API ABattleManager : public AActor
{
	GENERATED_BODY()

public:
	ABattleManager();

	// ---- Settings (editable when the manager is placed in a level) ----

	UPROPERTY(EditAnywhere, Category = "Battle")
	int64 BattleSeed = 1;

	// Multiplies every group's ship count (1 = 82 ships and 6 structures).
	UPROPERTY(EditAnywhere, Category = "Battle", meta = (ClampMin = 1, ClampMax = 100))
	int32 BattleScale = 1;

	// Let the built-in commander play blue too (watch mode).
	UPROPERTY(EditAnywhere, Category = "Battle")
	bool bBlueAI = false;

	// In watch mode, still stop at every pulse.
	UPROPERTY(EditAnywhere, Category = "Battle")
	bool bPauseAtPulseWhenWatching = false;

	UPROPERTY(EditAnywhere, Category = "Battle", meta = (ClampMin = 1, ClampMax = 64))
	int32 SimThreads = 4;

	// Playback speed; 1 = real time.
	UPROPERTY(EditAnywhere, Category = "Battle", meta = (ClampMin = 0.1, ClampMax = 16))
	float SimSpeed = 1.f;

	// Heights of the three layers and the terrain floor, in distance units
	// (1 unit = 1 Unreal unit).
	UPROPERTY(EditAnywhere, Category = "Look")
	float LowLayerZ = -900.f;
	UPROPERTY(EditAnywhere, Category = "Look")
	float MainLayerZ = 0.f;
	UPROPERTY(EditAnywhere, Category = "Look")
	float FighterLayerZ = 800.f;
	UPROPERTY(EditAnywhere, Category = "Look")
	float FloorZ = -1500.f;

	// Ships are drawn this much larger than their collision size so they
	// read at fleet zoom.
	UPROPERTY(EditAnywhere, Category = "Look")
	float ShipScale = 2.f;

	UPROPERTY(EditAnywhere, Category = "Look")
	float KeyLightLux = 3.f;
	UPROPERTY(EditAnywhere, Category = "Look")
	float FillLightLux = 0.8f;
	UPROPERTY(EditAnywhere, Category = "Look")
	float ExposureBias = 0.f;

	// Thin lines from every ship down to the floor, to read heights.
	UPROPERTY(EditAnywhere, Category = "Look")
	bool bDropLines = true;

	// ---- Used by the player controller and the HUD ----

	EBattlePhase GetPhase() const { return Phase; }
	const vela::Snapshot& Current() const { return Cur; }
	const std::vector<vela::ShipInfo>& Ships() const { return Sim.ships(); }
	int32 NumGroups() const { return GroupNames.Num(); }
	const FString& GroupName(int32 Group) const;
	const FString& CoreError() const { return ErrorText; }
	const FString& Report() const { return ReportText; }

	// The side the player commands.
	static constexpr int32 PlayerSide = 0;

	// Sim plane (x, y) at height z to world, and back.
	FVector SimToWorld(float X, float Y, float Z) const;
	FVector2D WorldToSim(const FVector& World) const;
	float LayerZ(uint8 Layer) const;

	// Where things are drawn this frame.
	FVector GroupWorld(int32 Group) const;
	const TArray<FVector>& ShipWorld() const { return ShipPos; }
	bool ShipShown(int32 Ship) const { return ShipVisible.IsValidIndex(Ship) && ShipVisible[Ship]; }

	int32 SelectedGroup = -1;

	// Gives an order to one of the player's groups. Only works while planning.
	// Returns BC_OK or an error code; the message is shown on the HUD.
	int32 IssueOrder(int32 Group, int32 Verb, int32 A = 0, int32 B = 0);

	// Plays the next pulse.
	void Go();
	void Restart(int64 NewSeed);
	void SetBlueAI(bool bAI);
	void ShowMessage(const FString& Text);
	const FString& Message() const { return MessageText; }
	double MessageAge() const;

protected:
	virtual void BeginPlay() override;
	virtual void EndPlay(const EEndPlayReason::Type Reason) override;
	virtual void Tick(float DeltaSeconds) override;

private:
	struct FShot
	{
		uint32 From, To;
		uint8 Side, DamageType;
		bool bHit, bSalvo;
		float Age, Life;
		FVector MissOffset;
	};
	struct FFlash
	{
		FVector Pos;
		float Size, Age, Life;
		uint8 Kind; // 0 fire, 1 shield, 2 jump
	};
	struct FRing
	{
		FVector Center;
		float Radius, Age, Life;
		FLinearColor Color;
	};

	// Setup
	void SetupLook();
	bool StartBattle();
	UInstancedStaticMeshComponent* MakeInstances(UStaticMesh* Mesh, const FLinearColor& Color, bool bShadows);
	void BuildShips();
	void BuildTerrain(const std::vector<uint8_t>& Cells);
	void DrawStaticLines(const std::vector<uint8_t>& Cells);

	// Simulation flow
	bool StopsAtPulse() const;
	void HarvestWorker();
	void PumpWorker();
	void Advance(float DeltaSeconds);
	void OnStep(const vela::Snapshot& Before, const vela::Snapshot& After);
	void WaitForWorker();

	// Drawing
	void UpdateShips(float DeltaSeconds);
	void UpdateFighters();
	void UpdateMissiles();
	void UpdateEffects(float DeltaSeconds);
	void DrawFrameLines();
	void AddFlash(const FVector& Pos, float Size, uint8 Kind, float Life = 0.6f);
	void AddRing(const FVector& Center, float Radius, const FLinearColor& Color, float Life);
	float ShipLength(int32 Ship) const;
	float LineWidth(const FVector& At, float Pixels) const;
	void DrawCircle(const FVector& Center, float Radius, const FLinearColor& Color, float Width, int32 Segments = 48);
	static void SetPool(UInstancedStaticMeshComponent* Pool, const TArray<FTransform>& Transforms);

	// Core and threading
	vela::Battle Sim;
	std::deque<vela::Snapshot> Ahead;
	TFuture<std::vector<vela::Snapshot>> Pending;
	bool bWorkerParked = false;

	vela::Snapshot Prev, Cur;
	float Accum = 0.f;
	float Alpha = 1.f;
	EBattlePhase Phase = EBattlePhase::Failed;

	TArray<FString> GroupNames;
	FString ErrorText, ReportText, MessageText;
	double MessageTime = -1000.0;

	// Engine content
	UPROPERTY()
	TObjectPtr<UStaticMesh> CubeMesh;
	UPROPERTY()
	TObjectPtr<UStaticMesh> SphereMesh;
	UPROPERTY()
	TObjectPtr<UStaticMesh> CylinderMesh;
	UPROPERTY()
	TObjectPtr<UStaticMesh> ConeMesh;
	UPROPERTY()
	TObjectPtr<UMaterialInterface> BaseMaterial;

	// Ships: one instance set per (class, side); ShipSlot maps ship -> (set, index).
	UPROPERTY()
	TArray<TObjectPtr<UInstancedStaticMeshComponent>> ShipSets;
	TArray<FIntPoint> ShipSlot;
	TArray<FVector> ShipPos;
	TArray<float> ShipZ;
	TArray<bool> ShipVisible;

	// Pools
	UPROPERTY()
	TArray<TObjectPtr<UInstancedStaticMeshComponent>> FighterSets; // per side
	UPROPERTY()
	TArray<TObjectPtr<UInstancedStaticMeshComponent>> MissileSets; // per side
	UPROPERTY()
	TArray<TObjectPtr<UInstancedStaticMeshComponent>> FlashSets; // per flash kind

	// Terrain
	UPROPERTY()
	TArray<TObjectPtr<UInstancedStaticMeshComponent>> TerrainSets;

	UPROPERTY()
	TObjectPtr<ULineBatchComponent> StaticLines;
	UPROPERTY()
	TObjectPtr<ULineBatchComponent> FrameLines;

	TArray<FShot> Shots;
	TArray<FFlash> Flashes;
	TArray<FRing> Rings;
	TMap<uint32, FVector2D> StrikeMarks;
	FVector CameraPos = FVector::ZeroVector;
	float Clock = 0.f;
};
