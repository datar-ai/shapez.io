#include "BattleCameraPawn.h"

#include "Camera/CameraComponent.h"

ABattleCameraPawn::ABattleCameraPawn()
{
	Camera = CreateDefaultSubobject<UCameraComponent>(TEXT("Camera"));
	RootComponent = Camera;
	Camera->FieldOfView = 60.f;
	Camera->bUsePawnControlRotation = false;
	SetActorEnableCollision(false);
}
