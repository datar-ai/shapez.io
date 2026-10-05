#pragma once

#include "CoreMinimal.h"
#include "GameFramework/HUD.h"

#include "BattleHUD.generated.h"

class ABattleManager;
class UFont;

// Canvas HUD: pulse and command points, the group list, labels over groups,
// messages and the after-action report. No UMG assets needed.
UCLASS(Config = Game)
class VELABATTLE_API ABattleHUD : public AHUD
{
	GENERATED_BODY()

public:
	virtual void DrawHUD() override;

	// Group whose row in the list is at this screen point, -1 when none.
	int32 RowAt(const FVector2D& Point) const;

	// Set in Config/DefaultGame.ini; needs Traditional Chinese glyphs.
	UPROPERTY(Config)
	FSoftObjectPath FontPath;

protected:
	virtual void BeginPlay() override;

private:
	ABattleManager* Manager();
	void Text(const FString& S, float X, float Y, const FLinearColor& Color, float Size = 1.f);
	void Bar(float X, float Y, float W, float H, float Fill, const FLinearColor& Color);
	void DrawTopBar(ABattleManager* M);
	void DrawGroupList(ABattleManager* M);
	void DrawWorldLabels(ABattleManager* M);
	void DrawHelp(ABattleManager* M);

	UPROPERTY()
	TObjectPtr<UFont> Font;

	TWeakObjectPtr<ABattleManager> CachedManager;
	TArray<TPair<FBox2D, int32>> Rows;
};
