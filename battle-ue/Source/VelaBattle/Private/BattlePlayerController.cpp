#include "BattlePlayerController.h"

#include "BattleHUD.h"
#include "BattleManager.h"
#include "EngineUtils.h"
#include "GameFramework/Pawn.h"
#include "InputCoreTypes.h"

ABattlePlayerController::ABattlePlayerController()
{
	bShowMouseCursor = true;
	bEnableClickEvents = false;
	DefaultMouseCursor = EMouseCursor::Default;
}

void ABattlePlayerController::BeginPlay()
{
	Super::BeginPlay();
	FInputModeGameAndUI Mode;
	Mode.SetHideCursorDuringCapture(false);
	Mode.SetLockMouseToViewportBehavior(EMouseLockMode::DoNotLock);
	SetInputMode(Mode);
}

ABattleManager* ABattlePlayerController::Manager()
{
	if (!CachedManager.IsValid())
	{
		TActorIterator<ABattleManager> It(GetWorld());
		CachedManager = It ? *It : nullptr;
	}
	return CachedManager.Get();
}

bool ABattlePlayerController::Pressed(const FKey& Key)
{
	return WasInputKeyJustPressed(Key);
}

void ABattlePlayerController::PlayerTick(float DeltaTime)
{
	Super::PlayerTick(DeltaTime);
	UpdateCamera(DeltaTime);
	if (ABattleManager* M = Manager())
	{
		if (M->GetPhase() != EBattlePhase::Failed)
		{
			HandleMouse();
			HandleKeys();
		}
	}
	float MX = 0.f, MY = 0.f;
	if (GetMousePosition(MX, MY))
	{
		LastMouse = FVector2D(MX, MY);
	}
}

void ABattlePlayerController::UpdateCamera(float DeltaTime)
{
	// Turn and tilt.
	const float TurnRate = 90.f * DeltaTime;
	if (IsInputKeyDown(EKeys::Q)) CamYaw += TurnRate;
	if (IsInputKeyDown(EKeys::E)) CamYaw -= TurnRate;
	if (IsInputKeyDown(EKeys::PageUp)) CamPitch -= TurnRate * 0.6f;
	if (IsInputKeyDown(EKeys::PageDown)) CamPitch += TurnRate * 0.6f;
	float MX = 0.f, MY = 0.f;
	if (IsInputKeyDown(EKeys::MiddleMouseButton) && GetMousePosition(MX, MY))
	{
		CamYaw += (MX - LastMouse.X) * 0.25f;
		CamPitch -= (MY - LastMouse.Y) * 0.2f;
	}
	CamPitch = FMath::Clamp(CamPitch, -89.f, -8.f);

	// Zoom toward the focus point.
	float Wheel = 0.f;
	if (WasInputKeyJustPressed(EKeys::MouseScrollUp)) Wheel += 1.f;
	if (WasInputKeyJustPressed(EKeys::MouseScrollDown)) Wheel -= 1.f;
	CamDistance = FMath::Clamp(CamDistance * FMath::Pow(0.85f, Wheel), 800.f, 80000.f);

	// Pan on the ground plane, faster when zoomed out.
	FVector2D Move = FVector2D::ZeroVector;
	if (IsInputKeyDown(EKeys::W) || IsInputKeyDown(EKeys::Up)) Move.X += 1.f;
	if (IsInputKeyDown(EKeys::S) || IsInputKeyDown(EKeys::Down)) Move.X -= 1.f;
	if (IsInputKeyDown(EKeys::D) || IsInputKeyDown(EKeys::Right)) Move.Y += 1.f;
	if (IsInputKeyDown(EKeys::A) || IsInputKeyDown(EKeys::Left)) Move.Y -= 1.f;
	if (!Move.IsZero())
	{
		const float Speed = CamDistance * 0.9f * (IsInputKeyDown(EKeys::LeftShift) ? 2.5f : 1.f);
		const FRotator Flat(0.f, CamYaw, 0.f);
		const FVector Forward = Flat.Vector();
		const FVector Right = FRotationMatrix(Flat).GetUnitAxis(EAxis::Y);
		Focus += (Forward * Move.X + Right * Move.Y).GetSafeNormal2D() * Speed * DeltaTime;
		Focus.X = FMath::Clamp(Focus.X, -20000.f, 20000.f);
		Focus.Y = FMath::Clamp(Focus.Y, -20000.f, 20000.f);
	}
	if (WasInputKeyJustPressed(EKeys::Home))
	{
		Focus = FVector::ZeroVector;
		CamYaw = 0.f;
		CamPitch = -55.f;
		CamDistance = 26000.f;
	}

	if (APawn* P = GetPawn())
	{
		const FRotator View(CamPitch, CamYaw, 0.f);
		P->SetActorLocationAndRotation(Focus - View.Vector() * CamDistance, View);
	}
}

void ABattlePlayerController::Frame(const FVector& Center, float Distance)
{
	Focus = Center;
	CamDistance = Distance;
}

int32 ABattlePlayerController::GroupUnderCursor(const FVector2D& Mouse)
{
	ABattleManager* M = Manager();
	if (!M)
	{
		return -1;
	}
	const TArray<FVector>& Pos = M->ShipWorld();
	const auto& Info = M->Ships();
	float Best = 14.f * 14.f;
	int32 BestGroup = -1;
	for (int32 I = 0; I < Pos.Num(); ++I)
	{
		FVector2D Screen;
		if (M->ShipShown(I) && ProjectWorldLocationToScreen(Pos[I], Screen, false))
		{
			const float D = float(FVector2D::DistSquared(Screen, Mouse));
			if (D < Best)
			{
				Best = D;
				BestGroup = Info[I].group;
			}
		}
	}
	if (BestGroup >= 0)
	{
		return BestGroup;
	}
	Best = 40.f * 40.f;
	const auto& Groups = M->Current().groups;
	for (int32 G = 0; G < int32(Groups.size()); ++G)
	{
		if (Groups[G].alive == 0)
		{
			continue;
		}
		FVector2D Screen;
		if (ProjectWorldLocationToScreen(M->GroupWorld(G), Screen, false))
		{
			const float D = float(FVector2D::DistSquared(Screen, Mouse));
			if (D < Best)
			{
				Best = D;
				BestGroup = G;
			}
		}
	}
	return BestGroup;
}

void ABattlePlayerController::HandleMouse()
{
	ABattleManager* M = Manager();
	float MX = 0.f, MY = 0.f;
	if (!GetMousePosition(MX, MY))
	{
		return;
	}
	const FVector2D Mouse(MX, MY);

	if (Pressed(EKeys::LeftMouseButton))
	{
		int32 Group = -1;
		if (ABattleHUD* Hud = Cast<ABattleHUD>(GetHUD()))
		{
			Group = Hud->RowAt(Mouse);
		}
		if (Group < 0)
		{
			Group = GroupUnderCursor(Mouse);
		}
		M->SelectedGroup = Group;
	}

	if (Pressed(EKeys::RightMouseButton))
	{
		const int32 Sel = M->SelectedGroup;
		const auto& Groups = M->Current().groups;
		if (Sel < 0 || Sel >= int32(Groups.size()) || Groups[Sel].side != ABattleManager::PlayerSide)
		{
			M->ShowMessage(TEXT("先選一個藍方戰鬥群"));
			return;
		}
		const int32 Under = GroupUnderCursor(Mouse);
		if (Under >= 0 && Groups[Under].side != ABattleManager::PlayerSide)
		{
			int32 Verb = BC_VERB_ATTACK;
			if (IsInputKeyDown(EKeys::LeftControl) || IsInputKeyDown(EKeys::RightControl)) Verb = BC_VERB_FLANK_LEFT;
			if (IsInputKeyDown(EKeys::LeftAlt) || IsInputKeyDown(EKeys::RightAlt)) Verb = BC_VERB_FLANK_RIGHT;
			M->IssueOrder(Sel, Verb, Under);
			return;
		}
		if (Under >= 0)
		{
			return; // own group: nothing to do
		}
		// Open space: advance to the point under the cursor on the group's layer.
		FVector Origin, Dir;
		if (DeprojectMousePositionToWorld(Origin, Dir) && FMath::Abs(Dir.Z) > 1e-3f)
		{
			const double PlaneZ = M->GroupWorld(Sel).Z;
			const double T = (PlaneZ - Origin.Z) / Dir.Z;
			if (T > 0.f)
			{
				const FVector2D At = M->WorldToSim(Origin + Dir * T);
				M->IssueOrder(Sel, BC_VERB_ADVANCE, FMath::RoundToInt32(At.X), FMath::RoundToInt32(At.Y));
			}
		}
	}
}

void ABattlePlayerController::HandleKeys()
{
	ABattleManager* M = Manager();
	const int32 Sel = M->SelectedGroup;

	if (Pressed(EKeys::SpaceBar)) M->Go();
	if (Pressed(EKeys::P)) M->SetBlueAI(!M->bBlueAI);
	if (Pressed(EKeys::N)) M->Restart(M->BattleSeed + 1);
	if (Pressed(EKeys::B)) M->bDropLines = !M->bDropLines;
	if (Pressed(EKeys::Equals) || Pressed(EKeys::Add)) M->SimSpeed = FMath::Min(M->SimSpeed * 2.f, 16.f);
	if (Pressed(EKeys::Hyphen) || Pressed(EKeys::Subtract)) M->SimSpeed = FMath::Max(M->SimSpeed * 0.5f, 0.25f);
	if (Pressed(EKeys::F) && Sel >= 0) Frame(M->GroupWorld(Sel), FMath::Min(CamDistance, 9000.f));

	if (Pressed(EKeys::Tab))
	{
		// Next of the player's groups still able to take orders.
		const auto& Groups = M->Current().groups;
		const int32 N = int32(Groups.size());
		for (int32 Step = 1; Step <= N; ++Step)
		{
			const int32 G = ((Sel < 0 ? -1 : Sel) + Step + N) % N;
			const BcGroup& Gr = Groups[G];
			if (Gr.side == ABattleManager::PlayerSide && !Gr.fixed && Gr.alive > 0 &&
				(Gr.status == BC_GROUP_ACTIVE || Gr.status == BC_GROUP_RESERVE))
			{
				M->SelectedGroup = G;
				break;
			}
		}
	}

	struct FOrderKey
	{
		FKey Key;
		int32 Verb;
		int32 A;
	};
	const FOrderKey Orders[] = {
		{EKeys::H, BC_VERB_HOLD, 0},
		{EKeys::R, BC_VERB_RETREAT, 0},
		{EKeys::T, BC_VERB_SIGNATURE, 0},
		{EKeys::L, BC_VERB_CHANGE_LAYER, 0},
		{EKeys::C, BC_VERB_COMMIT_RESERVE, 0},
		{EKeys::One, BC_VERB_SET_DOCTRINE, 0},
		{EKeys::Two, BC_VERB_SET_DOCTRINE, 1},
		{EKeys::Three, BC_VERB_SET_DOCTRINE, 2},
		{EKeys::Five, BC_VERB_FOCUS_PART, BC_PART_ENGINE},
		{EKeys::Six, BC_VERB_FOCUS_PART, BC_PART_WEAPONS},
		{EKeys::Seven, BC_VERB_FOCUS_PART, BC_PART_SHIELD_GEN},
		{EKeys::Eight, BC_VERB_FOCUS_PART, BC_PART_BRIDGE},
	};
	for (const FOrderKey& O : Orders)
	{
		if (Pressed(O.Key))
		{
			M->IssueOrder(Sel, O.Verb, O.A);
		}
	}
}
