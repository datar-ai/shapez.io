#pragma once

#include "CoreMinimal.h"
#include "GameFramework/Pawn.h"

#include "BattleCameraPawn.generated.h"

class UCameraComponent;

// Just a camera; ABattlePlayerController moves it.
UCLASS()
class VELABATTLE_API ABattleCameraPawn : public APawn
{
	GENERATED_BODY()

public:
	ABattleCameraPawn();

	UPROPERTY(VisibleAnywhere, Category = "Camera")
	TObjectPtr<UCameraComponent> Camera;
};
