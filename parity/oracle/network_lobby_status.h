// Bounded client lobby-entry fixture for the complete production
// C4Network2::CheckStatusReached (7d43b47b src/C4Network2.cpp:2017-2057).
// Only surrounding control/transport state is supplied here; the decision
// and acknowledgement order execute the mechanically extracted C++ body.
namespace network_lobby_status
{
#include "network_game_state.inc"

struct StatusData
{
    C4NetGameState state;
    int target = -1;
    C4NetGameState getState() const { return state; }
    int getTargetCtrlTick() const { return target; }
    void SetTargetTick(int tick) { target = tick; }
};

struct GameData
{
    bool IsRunning = false;
    int HaltCount = 0;
    struct ControlData
    {
        int ControlTick = 23;
        bool CtrlTickReached(int tick) const { return ControlTick >= tick; }
    } Control;
} Game;

struct ConsoleData
{
    void UpdateHaltCtrls(bool) {}
} Console;

struct NetworkControl
{
    bool CtrlReady(int) const { return false; }
    void SetRunning(bool, int) {}
};

constexpr int PID_StatusAck = 1;
StatusData MkC4NetIOPacket(int, StatusData status) { return status; }

struct C4Network2
{
    bool fStatusReached = false;
    bool fLobbyRunning = false;
    bool fChasing = false;
    bool fDelayedActivateReq = false;
    StatusData Status;
    NetworkControl control;
    NetworkControl *pControl = &control;
    struct ClientList
    {
        int acknowledgements = 0;
        int target = -1;
        void SendMsgToHost(StatusData status)
        {
            ++acknowledgements;
            target = status.target;
        }
    } Clients;
    bool isHost() const { return false; }
    void OnStatusReached() {}
    void CheckStatusAck() {}
    void RequestActivate() {}
    void CheckStatusReached(bool fFromFinalInit);
};

#include "network_check_status_reached.inc"

void printCases()
{
    printf("\"network_lobby_status_reach\":[");
    bool first = true;
    for (const auto state : {GS_Lobby, GS_Pause, GS_Go})
        for (const bool lobby_running : {false, true})
        {
            C4Network2 network;
            network.Status.state = state;
            network.fLobbyRunning = lobby_running;
            network.CheckStatusReached(false);
            if (!first) printf(",");
            first = false;
            printf("{\"state\":%d,\"lobby_running\":%s,\"control_tick\":%d,"
                   "\"acknowledgements\":%d,\"ack_target\":%d}",
                   state, lobby_running ? "true" : "false", Game.Control.ControlTick,
                   network.Clients.acknowledgements, network.Clients.target);
        }
    printf("]");
}
}
