#include "BattleHUD.h"

#include "BattleManager.h"
#include "Engine/Canvas.h"
#include "Engine/Engine.h"
#include "Engine/Font.h"
#include "EngineUtils.h"

namespace
{
	const FLinearColor HudBlue(0.35f, 0.65f, 1.f);
	const FLinearColor HudRed(1.f, 0.45f, 0.35f);
	const FLinearColor HudDim(0.6f, 0.6f, 0.65f);
	const FLinearColor HudPanel(0.f, 0.f, 0.f, 0.55f);

	FLinearColor SideTint(uint8 Side)
	{
		return Side == 0 ? HudBlue : HudRed;
	}

	const TCHAR* StatusName(const BcGroup& G)
	{
		switch (G.status)
		{
		case BC_GROUP_ACTIVE:
		{
			static const TCHAR* Phases[] = {TEXT("接近"), TEXT("交戰"), TEXT("脫離"), TEXT("整補")};
			return G.fixed ? TEXT("建築") : Phases[G.phase & 3];
		}
		case BC_GROUP_RESERVE: return TEXT("預備");
		case BC_GROUP_RETREATING: return TEXT("撤離中");
		case BC_GROUP_ROUTED: return TEXT("潰散");
		case BC_GROUP_ESCAPED: return TEXT("已撤出");
		default: return TEXT("全滅");
		}
	}

	FString SignatureText(const BcGroup& G)
	{
		if (G.signature < 0) return FString::Printf(TEXT("招牌技充能 %.1f 秒"), -G.signature / float(BC_TICK_HZ));
		if (G.signature_armed) return TEXT("招牌技待發");
		if (G.signature == 0) return TEXT("招牌技就緒");
		return FString::Printf(TEXT("招牌技冷卻 %d 秒"), FMath::CeilToInt(G.signature / float(BC_TICK_HZ)));
	}

	const TCHAR* DoctrineName(uint8 D)
	{
		static const TCHAR* Names[] = {TEXT("砲列"), TEXT("鐵砧"), TEXT("鐵鎚")};
		return Names[FMath::Min<int32>(D, 2)];
	}

	const TCHAR* PartName(uint8 P)
	{
		switch (P)
		{
		case BC_PART_ENGINE: return TEXT("集火引擎");
		case BC_PART_WEAPONS: return TEXT("集火武器");
		case BC_PART_SHIELD_GEN: return TEXT("集火護盾產生器");
		case BC_PART_BRIDGE: return TEXT("集火艦橋");
		default: return TEXT("");
		}
	}

	FString LayerName(uint8 Layer)
	{
		FString S = (Layer & 0x7f) == BC_LAYER_LOW ? TEXT("下層") : TEXT("主戰層");
		if (Layer & BC_LAYER_MOVING) S += TEXT("（換層中）");
		return S;
	}
}

void ABattleHUD::BeginPlay()
{
	Super::BeginPlay();
	Font = Cast<UFont>(FontPath.TryLoad());
	if (!Font && GEngine)
	{
		Font = GEngine->GetMediumFont();
	}
}

ABattleManager* ABattleHUD::Manager()
{
	if (!CachedManager.IsValid())
	{
		TActorIterator<ABattleManager> It(GetWorld());
		CachedManager = It ? *It : nullptr;
	}
	return CachedManager.Get();
}

void ABattleHUD::Text(const FString& S, float X, float Y, const FLinearColor& Color, float Size)
{
	DrawText(S, Color, X, Y, Font, Size);
}

void ABattleHUD::Bar(float X, float Y, float W, float H, float Fill, const FLinearColor& Color)
{
	DrawRect(FLinearColor(0.f, 0.f, 0.f, 0.6f), X, Y, W, H);
	DrawRect(Color, X, Y, W * FMath::Clamp(Fill, 0.f, 1.f), H);
}

int32 ABattleHUD::RowAt(const FVector2D& Point) const
{
	for (const TPair<FBox2D, int32>& Row : Rows)
	{
		if (Row.Key.IsInside(Point))
		{
			return Row.Value;
		}
	}
	return -1;
}

void ABattleHUD::DrawHUD()
{
	Super::DrawHUD();
	Rows.Reset();
	ABattleManager* M = Manager();
	if (!M || !Canvas)
	{
		return;
	}
	if (M->GetPhase() == EBattlePhase::Failed)
	{
		Text(M->CoreError(), 40.f, 40.f, HudRed, 1.4f);
		return;
	}
	DrawWorldLabels(M);
	DrawTopBar(M);
	DrawGroupList(M);
	DrawHelp(M);

	if (M->MessageAge() < 3.0 && !M->Message().IsEmpty())
	{
		float W = 0.f, H = 0.f;
		GetTextSize(M->Message(), W, H, Font, 1.2f);
		const float X = (Canvas->SizeX - W) * 0.5f, Y = Canvas->SizeY * 0.72f;
		DrawRect(HudPanel, X - 12.f, Y - 6.f, W + 24.f, H + 12.f);
		Text(M->Message(), X, Y, FLinearColor(1.f, 0.85f, 0.4f), 1.2f);
	}

	if (M->GetPhase() == EBattlePhase::Over)
	{
		TArray<FString> Lines;
		M->Report().ParseIntoArrayLines(Lines);
		const float X = Canvas->SizeX * 0.5f - 330.f, Y0 = Canvas->SizeY * 0.2f;
		DrawRect(FLinearColor(0.f, 0.f, 0.f, 0.8f), X - 16.f, Y0 - 16.f, 700.f, 32.f + 20.f * (Lines.Num() + 2));
		Text(TEXT("戰報"), X, Y0, FLinearColor::White, 1.3f);
		for (int32 K = 0; K < Lines.Num(); ++K)
		{
			Text(Lines[K], X, Y0 + 30.f + 20.f * K, HudDim);
		}
		Text(TEXT("按 N 開新的一場"), X, Y0 + 40.f + 20.f * Lines.Num(), FLinearColor(1.f, 0.85f, 0.4f));
	}
}

void ABattleHUD::DrawTopBar(ABattleManager* M)
{
	const vela::Snapshot& S = M->Current();
	DrawRect(HudPanel, 0.f, 0.f, Canvas->SizeX, 34.f);
	const float Seconds = float(S.tick % BC_PULSE_TICKS) / BC_TICK_HZ;
	FString State;
	switch (M->GetPhase())
	{
	case EBattlePhase::Planning:
		State = M->bBlueAI ? TEXT("脈衝暫停：按空白鍵繼續")
						   : TEXT("脈衝暫停：選藍方戰鬥群下令，按空白鍵開打 60 秒");
		break;
	case EBattlePhase::Running:
		State = FString::Printf(TEXT("戰鬥中  %.0f / 60 秒  速度 ×%g"), Seconds, M->SimSpeed);
		break;
	case EBattlePhase::Over:
		State = S.winner == 0 ? TEXT("藍方勝") : S.winner == 1 ? TEXT("紅方勝") : TEXT("平手");
		break;
	default:
		break;
	}
	const FString Line = FString::Printf(TEXT("種子 %lld   脈衝 %u   指令點 藍 %d ／ 紅 %d   %s%s"),
		M->BattleSeed, uint32(S.pulse()), S.command_points[0], S.command_points[1], *State,
		M->bBlueAI ? TEXT("   〔藍方 AI 指揮〕") : TEXT(""));
	Text(Line, 14.f, 8.f, FLinearColor::White, 1.f);
}

void ABattleHUD::DrawGroupList(ABattleManager* M)
{
	const vela::Snapshot& S = M->Current();
	const float X = 10.f, W = 330.f, RowH = 38.f;
	float Y = 44.f;
	for (int32 Side = 0; Side < 2; ++Side)
	{
		Text(Side == 0 ? TEXT("藍方（你）") : TEXT("紅方（AI）"), X, Y, SideTint(uint8(Side)), 1.f);
		Y += 20.f;
		for (int32 G = 0; G < int32(S.groups.size()); ++G)
		{
			const BcGroup& Gr = S.groups[G];
			if (Gr.side != Side)
			{
				continue;
			}
			const bool bGone = Gr.alive == 0 || Gr.status == BC_GROUP_ESCAPED || Gr.status == BC_GROUP_DESTROYED;
			const bool bSelected = G == M->SelectedGroup;
			DrawRect(bSelected ? FLinearColor(0.35f, 0.3f, 0.05f, 0.75f) : HudPanel, X, Y, W, RowH - 3.f);
			const FLinearColor Tint = bGone ? HudDim * 0.7f : SideTint(Gr.side);
			Text(FString::Printf(TEXT("%s  %u/%u  %s"), *M->GroupName(G), uint32(Gr.alive), uint32(Gr.ships), StatusName(Gr)), X + 6.f, Y + 2.f, Tint, 0.9f);
			if (!bGone && !Gr.fixed)
			{
				FString Detail = FString::Printf(TEXT("%s  %s  %s"), DoctrineName(Gr.doctrine), *LayerName(Gr.layer), *SignatureText(Gr));
				if (Gr.fighters_out + Gr.fighters_docked > 0)
				{
					Detail += FString::Printf(TEXT("  戰機 %u/%u"), uint32(Gr.fighters_out), uint32(Gr.fighters_out) + uint32(Gr.fighters_docked));
				}
				if (Gr.focus_part)
				{
					Detail += TEXT("  ");
					Detail += PartName(Gr.focus_part);
				}
				Text(Detail, X + 6.f, Y + 19.f, HudDim, 0.75f);
				Bar(X + W - 64.f, Y + 6.f, 56.f, 6.f, Gr.cohesion / 1000.f, FLinearColor(0.3f, 0.9f, 0.4f));
			}
			Rows.Add(TPair<FBox2D, int32>(FBox2D(FVector2D(X, Y), FVector2D(X + W, Y + RowH - 3.f)), G));
			Y += RowH;
		}
		Y += 8.f;
	}
}

void ABattleHUD::DrawWorldLabels(ABattleManager* M)
{
	const vela::Snapshot& S = M->Current();
	for (int32 G = 0; G < int32(S.groups.size()); ++G)
	{
		const BcGroup& Gr = S.groups[G];
		if (Gr.alive == 0 || Gr.status == BC_GROUP_ESCAPED || Gr.status == BC_GROUP_DESTROYED)
		{
			continue;
		}
		const FVector P = Project(M->GroupWorld(G) + FVector(0.f, 0.f, 350.f));
		if (P.Z <= 0.f)
		{
			continue;
		}
		const bool bSelected = G == M->SelectedGroup;
		const FLinearColor Tint = bSelected ? FLinearColor(1.f, 0.95f, 0.4f) : SideTint(Gr.side);
		float W = 0.f, H = 0.f;
		const FString& Name = M->GroupName(G);
		GetTextSize(Name, W, H, Font, 0.8f);
		Text(Name, float(P.X) - W * 0.5f, float(P.Y) - H - 6.f, Tint, 0.8f);
		if (!Gr.fixed)
		{
			Bar(float(P.X) - 24.f, float(P.Y) - 4.f, 48.f, 4.f, Gr.cohesion / 1000.f, FLinearColor(0.3f, 0.9f, 0.4f));
		}
	}
	for (const BcStrike& St : S.strikes)
	{
		const FVector P = Project(M->SimToWorld(float(St.x), float(St.y), 0.f));
		if (P.Z > 0.f)
		{
			Text(FString::Printf(TEXT("落點 %.1f"), St.ticks_left / float(BC_TICK_HZ)), float(P.X) - 20.f, float(P.Y) - 8.f, SideTint(St.side), 0.9f);
		}
	}
}

void ABattleHUD::DrawHelp(ABattleManager* M)
{
	const TCHAR* Lines[] = {
		TEXT("左鍵 選戰鬥群　右鍵敵群 攻擊（Ctrl 左側擊、Alt 右側擊）　右鍵空地 推進"),
		TEXT("H 固守　R 撤離　T 招牌技　L 換層　C 投入預備　1/2/3 準則　5-8 集火部位　Tab 下一群"),
		TEXT("空白鍵 開打　P 藍方交給 AI　N 新戰鬥　+/- 速度　B 高度線　WASD 平移　滾輪 縮放　Q/E 旋轉　中鍵拖曳 環繞　F 對準"),
	};
	const float H = 20.f;
	float Y = Canvas->SizeY - H * UE_ARRAY_COUNT(Lines) - 10.f;
	DrawRect(HudPanel, 0.f, Y - 6.f, Canvas->SizeX, H * UE_ARRAY_COUNT(Lines) + 16.f);
	for (const TCHAR* L : Lines)
	{
		Text(L, 14.f, Y, HudDim, 0.85f);
		Y += H;
	}
	(void)M;
}
