#pragma once

#include "CoreMinimal.h"
#include "GameFramework/PlayerController.h"

#include "BattlePlayerController.generated.h"

class ABattleManager;

// Camera (pan, zoom, orbit), group selection and orders.
//
// Keys are read by polling, so the project needs no input assets:
//   camera: WASD / arrows pan, wheel zoom, Q E turn, middle-drag orbit,
//           PageUp PageDown tilt, F focus selection, Home reset
//   select: left-click a ship, a group or a row of the group list; Tab cycles
//   orders (at a pulse pause): right-click an enemy = attack
//           (Ctrl flank left, Alt flank right), right-click open space = advance,
//           H hold, R retreat, T signature, L change layer, C commit reserve,
//           1 2 3 doctrine line / anvil / hammer, 5 6 7 8 focus engines /
//           weapons / shield generators / bridge
//   battle: Space play the next pulse, P hand blue to the AI and back,
//           N new battle, + - playback speed, B drop lines
UCLASS()
class VELABATTLE_API ABattlePlayerController : public APlayerController
{
	GENERATED_BODY()

public:
	ABattlePlayerController();

	virtual void PlayerTick(float DeltaTime) override;

	ABattleManager* Manager();

protected:
	virtual void BeginPlay() override;

private:
	void UpdateCamera(float DeltaTime);
	void HandleMouse();
	void HandleKeys();
	void Frame(const FVector& Center, float Distance);

	// Group under the cursor: a ship within a few pixels, else a group marker.
	int32 GroupUnderCursor(const FVector2D& Mouse);
	bool Pressed(const FKey& Key);

	TWeakObjectPtr<ABattleManager> CachedManager;

	FVector Focus = FVector::ZeroVector;
	float CamYaw = 0.f;
	float CamPitch = -55.f;
	float CamDistance = 26000.f;
	FVector2D LastMouse = FVector2D::ZeroVector;
};
