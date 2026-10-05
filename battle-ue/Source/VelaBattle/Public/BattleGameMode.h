#pragma once

#include "CoreMinimal.h"
#include "GameFramework/GameModeBase.h"

#include "BattleGameMode.generated.h"

// Sets up the battle in any map: camera pawn, controller, HUD, and a battle
// manager unless the level already has one (place one to change its settings).
UCLASS()
class VELABATTLE_API ABattleGameMode : public AGameModeBase
{
	GENERATED_BODY()

public:
	ABattleGameMode();

protected:
	virtual void BeginPlay() override;
};
