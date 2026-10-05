#include "BattleGameMode.h"

#include "BattleCameraPawn.h"
#include "BattleHUD.h"
#include "BattleManager.h"
#include "BattlePlayerController.h"
#include "EngineUtils.h"

ABattleGameMode::ABattleGameMode()
{
	DefaultPawnClass = ABattleCameraPawn::StaticClass();
	PlayerControllerClass = ABattlePlayerController::StaticClass();
	HUDClass = ABattleHUD::StaticClass();
}

void ABattleGameMode::BeginPlay()
{
	Super::BeginPlay();
	if (!TActorIterator<ABattleManager>(GetWorld()))
	{
		GetWorld()->SpawnActor<ABattleManager>(FVector::ZeroVector, FRotator::ZeroRotator);
	}
}
