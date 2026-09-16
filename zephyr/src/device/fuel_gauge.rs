// Copyright (c) 2026 Open Device Partnership and Contributors
// SPDX-License-Identifier: Apache-2.0

//! Device wrapper for a fuel gauge.

use crate::raw::fuel_gauge_prop_type;

/// A fuel gauge device.
pub struct FuelGauge {
    device: *const crate::raw::device,
}

#[rustfmt::skip]
#[repr(u32)]
pub(crate) enum FuelGaugeProp {
    AvgCurrentUa = fuel_gauge_prop_type::FUEL_GAUGE_AVG_CURRENT_UA,
    Cutoff = fuel_gauge_prop_type::FUEL_GAUGE_CHARGE_CUTOFF,
    CurrentUa = fuel_gauge_prop_type::FUEL_GAUGE_CURRENT_UA,
    CycleCount = fuel_gauge_prop_type::FUEL_GAUGE_CYCLE_COUNT,
    ConnectState = fuel_gauge_prop_type::FUEL_GAUGE_CONNECT_STATE,
    Flags = fuel_gauge_prop_type::FUEL_GAUGE_FLAGS,
    FullChargeCapacityUah = fuel_gauge_prop_type::FUEL_GAUGE_FULL_CHARGE_CAPACITY_UAH,
    PresentState = fuel_gauge_prop_type::FUEL_GAUGE_PRESENT_STATE,
    RemainingCapacityUah = fuel_gauge_prop_type::FUEL_GAUGE_REMAINING_CAPACITY_UAH,
    RuntimeToEmptyMins = fuel_gauge_prop_type::FUEL_GAUGE_RUNTIME_TO_EMPTY_MINS,
    RuntimeToFullMins = fuel_gauge_prop_type::FUEL_GAUGE_RUNTIME_TO_FULL_MINS,
    SbsMfrAccessWord = fuel_gauge_prop_type::FUEL_GAUGE_SBS_MFR_ACCESS,
    AbsoluteStateOfChargePct = fuel_gauge_prop_type::FUEL_GAUGE_ABSOLUTE_STATE_OF_CHARGE_PCT,
    RelativeStateOfChargePct = fuel_gauge_prop_type::FUEL_GAUGE_RELATIVE_STATE_OF_CHARGE_PCT,
    TemperatureDk = fuel_gauge_prop_type::FUEL_GAUGE_TEMPERATURE_DK,
    VoltageUv = fuel_gauge_prop_type::FUEL_GAUGE_VOLTAGE_UV,
    SbsMode = fuel_gauge_prop_type::FUEL_GAUGE_SBS_MODE,
    ChgCurrentUa = fuel_gauge_prop_type::FUEL_GAUGE_CHARGE_CURRENT_UA,
    ChgVoltageUv = fuel_gauge_prop_type::FUEL_GAUGE_CHARGE_VOLTAGE_UV,
    FgStatus = fuel_gauge_prop_type::FUEL_GAUGE_STATUS,
    DesignCap = fuel_gauge_prop_type::FUEL_GAUGE_DESIGN_CAPACITY,
    DesignVoltMv = fuel_gauge_prop_type::FUEL_GAUGE_DESIGN_VOLTAGE_MV,
    SbsAtRate = fuel_gauge_prop_type::FUEL_GAUGE_SBS_ATRATE,
    SbsAtRateTimeToFullMins = fuel_gauge_prop_type::FUEL_GAUGE_SBS_ATRATE_TIME_TO_FULL_MINS,
    SbsAtRateTimeToEmptyMins = fuel_gauge_prop_type::FUEL_GAUGE_SBS_ATRATE_TIME_TO_EMPTY_MINS,
    SbsAtRateOk = fuel_gauge_prop_type::FUEL_GAUGE_SBS_ATRATE_OK,
    SbsRemainingCapacityAlarm = fuel_gauge_prop_type::FUEL_GAUGE_SBS_REMAINING_CAPACITY_ALARM,
    SbsRemainingTimeAlarmMins = fuel_gauge_prop_type::FUEL_GAUGE_SBS_REMAINING_TIME_ALARM_MINS,
    CurrentDirection = fuel_gauge_prop_type::FUEL_GAUGE_CURRENT_DIRECTION,
    StateOfChargeAlarmPct = fuel_gauge_prop_type::FUEL_GAUGE_STATE_OF_CHARGE_ALARM_PCT,
    LowVoltageAlarmUv = fuel_gauge_prop_type::FUEL_GAUGE_LOW_VOLTAGE_ALARM_UV,
    HighVoltageAlarmUv = fuel_gauge_prop_type::FUEL_GAUGE_HIGH_VOLTAGE_ALARM_UV,
    LowCurrentAlarmUa = fuel_gauge_prop_type::FUEL_GAUGE_LOW_CURRENT_ALARM_UA,
    HighCurrentAlarmUa = fuel_gauge_prop_type::FUEL_GAUGE_HIGH_CURRENT_ALARM_UA,
    LowTemperatureAlarmDk = fuel_gauge_prop_type::FUEL_GAUGE_LOW_TEMPERATURE_ALARM_DK,
    HighTemperatureAlarmDk = fuel_gauge_prop_type::FUEL_GAUGE_HIGH_TEMPERATURE_ALARM_DK,
    GpioVoltageUv = fuel_gauge_prop_type::FUEL_GAUGE_GPIO_VOLTAGE_UV,
    LowGpioAlarmUv = fuel_gauge_prop_type::FUEL_GAUGE_LOW_GPIO_ALARM_UV,
    HighGpioAlarmUv = fuel_gauge_prop_type::FUEL_GAUGE_HIGH_GPIO_ALARM_UV,
    AdcMode = fuel_gauge_prop_type::FUEL_GAUGE_ADC_MODE,
    CcConfig = fuel_gauge_prop_type::FUEL_GAUGE_CC_CONFIG,
    StateOfHealth = fuel_gauge_prop_type::FUEL_GAUGE_STATE_OF_HEALTH,
    ThermVoltageUv = fuel_gauge_prop_type::FUEL_GAUGE_THERM_VOLTAGE_UV,
}

#[repr(u32)]
pub(crate) enum FuelGaugeBufferProp {
    ManufacturerName = crate::raw::fuel_gauge_prop_type::FUEL_GAUGE_MANUFACTURER_NAME,
    DeviceName = crate::raw::fuel_gauge_prop_type::FUEL_GAUGE_DEVICE_NAME,
    DeviceChemistry = crate::raw::fuel_gauge_prop_type::FUEL_GAUGE_DEVICE_CHEMISTRY,
}

// Fuel gauge buffer prop sizes
const FUEL_GAUGE_MANUFACTURER_NAME_SIZE: usize = 20;
const FUEL_GAUGE_DEVICE_NAME_SIZE: usize = 20;
const FUEL_GAUGE_DEVICE_CHEMISTRY_SIZE: usize = 4;

/// Maximum size of a `FuelGaugeString`. Strings may be smaller than this, but will not be larger than this, per the guarantees provided in the Zephyr docs.
/// This number does not contain the 1 "length" byte located at the beginning of the buffer props as indicated by the Zephyr docs.
const FUEL_GAUGE_STRING_MAX_SIZE: usize = 20;

/// A String returned by the fuel gauge API.
#[derive(Debug)]
pub struct FuelGaugeString {
    inner: heapless::String<FUEL_GAUGE_STRING_MAX_SIZE>,
}

impl FuelGaugeString {
    /// Returns the string as a `&str`.
    pub fn as_str(&self) -> &str {
        self.inner.as_str()
    }
}

impl AsRef<str> for FuelGaugeString {
    fn as_ref(&self) -> &str {
        self.inner.as_str()
    }
}

impl TryFrom<&[u8]> for FuelGaugeString {
    type Error = crate::error::Error;

    /// Parses a fuel gauge buffer property into a `FuelGaugeString`.
    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let len = *value
            .first()
            .ok_or(crate::error::Error(crate::raw::EINVAL))? as usize;
        if len > FUEL_GAUGE_STRING_MAX_SIZE {
            return Err(crate::error::Error(crate::raw::EINVAL));
        }

        let slice = value
            .get(1..1 + len)
            .ok_or(crate::error::Error(crate::raw::EINVAL))?;
        if !slice.is_ascii() {
            return Err(crate::error::Error(crate::raw::EILSEQ));
        }

        let str =
            core::str::from_utf8(slice).map_err(|_| crate::error::Error(crate::raw::EILSEQ))?;
        let mut inner = heapless::String::<FUEL_GAUGE_STRING_MAX_SIZE>::new();
        inner
            .push_str(str)
            .map_err(|_| crate::error::Error(crate::raw::EINVAL))?;

        Ok(FuelGaugeString { inner })
    }
}

/// Crate-internal API.
impl FuelGauge {
    /// Constructor, used by the devicetree generated code.
    pub(crate) unsafe fn new(
        unique: &crate::device::Unique,
        _static: &crate::device::NoStatic,
        device: *const crate::raw::device,
    ) -> Option<FuelGauge> {
        if !unique.once() {
            None
        } else {
            Some(FuelGauge { device })
        }
    }

    /// Private helper function to get a fuel gauge prop value.
    pub(crate) fn get_prop(
        &self,
        prop: FuelGaugeProp,
    ) -> crate::error::Result<crate::raw::fuel_gauge_prop_val> {
        let mut buffer = crate::raw::fuel_gauge_prop_val::default();

        crate::error::to_result_void(
            // SAFETY: - `self.device` lives for the entire duration of `self`.
            //         -  `prop` is a copy owned by this function.
            //         - `&mut buffer` is a valid pointer to a memory area of size `crate::raw::fuel_gauge_prop_val` on the stack.
            //            This memory area is local to get_prop(), so it will live as long as fuel_gauge_get_prop() is using it.
            //            In other words, by the time `buffer` goes out of scope, fuel_gauge_get_prop() will have already returned.
            unsafe { crate::raw::fuel_gauge_get_prop(self.device, prop as u16, &mut buffer) },
        )?;

        Ok(buffer)
    }

    /// Private helper function to set a fuel gauge prop value.
    pub(crate) fn set_prop(
        &self,
        prop: FuelGaugeProp,
        set: impl Fn(&mut crate::raw::fuel_gauge_prop_val),
    ) -> crate::error::Result<()> {
        let mut props: crate::raw::fuel_gauge_prop_val = crate::raw::fuel_gauge_prop_val::default();
        set(&mut props);

        crate::error::to_result_void(
            // SAFETY: - `self.device` lives for the entire duration of `self`.
            //         - `prop` is a copy owned by this function.
            //         - `props` is a copy owned by this function.
            unsafe { crate::raw::fuel_gauge_set_prop(self.device, prop as u16, props) },
        )
    }

    /// Private helper function to get a fuel gauge buffer prop value.
    pub(crate) fn get_buffer_prop(
        &self,
        prop: FuelGaugeBufferProp,
        buffer: &mut [u8],
    ) -> crate::error::Result<()> {
        crate::error::to_result_void(
            // SAFETY: - `self.device` lives for the entire duration of `self`.
            //         - `prop` is a copy owned by this function.
            //         - `buffer.as_mut_ptr()` is a valid pointer to a writable memory region
            //           of at least `buffer.len()` bytes that lives for the duration of this call.
            //         - The Zephyr API does not write beyond `buffer.len()` or retain the pointer after returning.
            unsafe {
                crate::raw::fuel_gauge_get_buffer_prop(
                    self.device,
                    prop as u16,
                    buffer.as_mut_ptr() as *mut core::ffi::c_void,
                    buffer.len(),
                )
            },
        )
    }
}

/// Public API.
impl FuelGauge {
    /// Verify that the device is ready for use.  At a minimum, this means the device has been
    /// successfully initialized.
    pub fn is_ready(&self) -> bool {
        // SAFETY: `self.device` lives for the entire duration of `self`.
        unsafe { crate::raw::device_is_ready(self.device) }
    }

    /// Reads the gauge's `manufacturer_name` into a string.
    pub fn manufacturer_name(&self) -> crate::error::Result<FuelGaugeString> {
        // Add +1 here since the first byte is the string length
        let mut buffer = const { [0u8; FUEL_GAUGE_MANUFACTURER_NAME_SIZE + 1] };
        self.get_buffer_prop(FuelGaugeBufferProp::ManufacturerName, &mut buffer)?;
        FuelGaugeString::try_from(buffer.as_slice())
    }

    /// Reads the gauge's `device_name` into a string.
    pub fn device_name(&self) -> crate::error::Result<FuelGaugeString> {
        // Add +1 here since the first byte is the string length
        let mut buffer = const { [0u8; FUEL_GAUGE_DEVICE_NAME_SIZE + 1] };
        self.get_buffer_prop(FuelGaugeBufferProp::DeviceName, &mut buffer)?;
        FuelGaugeString::try_from(buffer.as_slice())
    }

    /// Reads the gauge's `device_chemistry` into a string.
    pub fn device_chemistry(&self) -> crate::error::Result<FuelGaugeString> {
        // Add +1 here since the first byte is the string length
        let mut buffer = const { [0u8; FUEL_GAUGE_DEVICE_CHEMISTRY_SIZE + 1] };
        self.get_buffer_prop(FuelGaugeBufferProp::DeviceChemistry, &mut buffer)?;
        FuelGaugeString::try_from(buffer.as_slice())
    }

    /// Runtime Dynamic Battery Parameters.
    ///
    /// Provides a 1 minute average of the current on the battery. Does not check for flags or whether those values are bad readings. See driver instance header for details on implementation and how the average is calculated. Units in uA, negative=discharging.
    pub fn average_current(&self) -> crate::error::Result<i32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::AvgCurrentUa)
            .map(|val| unsafe { *val.avg_current_ua.as_ref() })
    }

    /// Whether the battery underlying the fuel-gauge is cut off from charge.
    pub fn cutoff(&self) -> crate::error::Result<bool> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::Cutoff)
            .map(|val| unsafe { *val.cutoff.as_ref() })
    }

    /// Battery current (uA); negative=discharging.
    pub fn current(&self) -> crate::error::Result<i32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::CurrentUa)
            .map(|val| unsafe { *val.current_ua.as_ref() })
    }

    /// Cycle count in charge/discharge cycles.
    pub fn cycle_count(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::CycleCount)
            .map(|val| unsafe { *val.cycle_count.as_ref() })
    }

    /// Connect state of battery.
    pub fn connect_state(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::ConnectState)
            .map(|val| unsafe { *val.connect_state.as_ref() })
    }

    /// General Error/Runtime Flags.
    pub fn flags(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::Flags)
            .map(|val| unsafe { *val.flags.as_ref() })
    }

    /// Full Charge Capacity in uAh (might change in some implementations to determine wear).
    pub fn full_charge_capacity(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::FullChargeCapacityUah)
            .map(|val| unsafe { *val.full_charge_capacity_uah.as_ref() })
    }

    /// Is the battery physically present.
    pub fn present_state(&self) -> crate::error::Result<bool> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::PresentState)
            .map(|val| unsafe { *val.present_state.as_ref() })
    }

    /// Remaining capacity in uAh.
    pub fn remaining_capacity(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::RemainingCapacityUah)
            .map(|val| unsafe { *val.remaining_capacity_uah.as_ref() })
    }

    /// Remaining battery life time in minutes.
    pub fn runtime_to_empty(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::RuntimeToEmptyMins)
            .map(|val| unsafe { *val.runtime_to_empty_mins.as_ref() })
    }

    /// Remaining time in minutes until battery reaches full charge.
    pub fn runtime_to_full(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::RuntimeToFullMins)
            .map(|val| unsafe { *val.runtime_to_full_mins.as_ref() })
    }

    /// Retrieve word from SBS1.1 ManufacturerAccess.
    pub fn sbs_mfr_access_word(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::SbsMfrAccessWord)
            .map(|val| unsafe { *val.sbs_mfr_access_word.as_ref() })
    }

    /// Absolute state of charge (percent, 0-100) - expressed as % of design capacity.
    pub fn absolute_state_of_charge(&self) -> crate::error::Result<u8> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::AbsoluteStateOfChargePct)
            .map(|val| unsafe { *val.absolute_state_of_charge_pct.as_ref() })
    }

    /// Relative state of charge (percent, 0-100) - expressed as % of full charge capacity.
    pub fn relative_state_of_charge(&self) -> crate::error::Result<u8> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::RelativeStateOfChargePct)
            .map(|val| unsafe { *val.relative_state_of_charge_pct.as_ref() })
    }

    /// Temperature in 0.1 K.
    pub fn temperature(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::TemperatureDk)
            .map(|val| unsafe { *val.temperature_dk.as_ref() })
    }

    /// Battery voltage (uV).
    pub fn voltage(&self) -> crate::error::Result<i32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::VoltageUv)
            .map(|val| unsafe { *val.voltage_uv.as_ref() })
    }

    /// Battery Mode (flags).
    pub fn sbs_mode(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::SbsMode)
            .map(|val| unsafe { *val.sbs_mode.as_ref() })
    }

    /// Sets the battery mode (flags).
    pub fn set_sbs_mode(&self, value: u16) -> crate::error::Result<()> {
        // SAFETY: `val` references initialized and properly-aligned union storage.
        self.set_prop(FuelGaugeProp::SbsMode, |val| unsafe {
            *val.sbs_mode.as_mut() = value
        })
    }

    /// Battery desired Max Charging Current (uA).
    pub fn charge_current(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::ChgCurrentUa)
            .map(|val| unsafe { *val.chg_current_ua.as_ref() })
    }

    /// Battery desired Max Charging Voltage (uV).
    pub fn charge_voltage(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::ChgVoltageUv)
            .map(|val| unsafe { *val.chg_voltage_uv.as_ref() })
    }

    /// Alarm, Status and Error codes (flags).
    pub fn status(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::FgStatus)
            .map(|val| unsafe { *val.fg_status.as_ref() })
    }

    /// Design Capacity (mAh or 10mWh).
    pub fn design_capacity(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::DesignCap)
            .map(|val| unsafe { *val.design_cap.as_ref() })
    }

    /// Design Voltage (mV).
    pub fn design_voltage(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::DesignVoltMv)
            .map(|val| unsafe { *val.design_volt_mv.as_ref() })
    }

    /// AtRate (mA or 10 mW).
    pub fn sbs_at_rate(&self) -> crate::error::Result<i16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::SbsAtRate)
            .map(|val| unsafe { *val.sbs_at_rate.as_ref() })
    }

    /// Sets the SBS AtRate (mA or 10 mW)
    pub fn set_sbs_at_rate(&self, value: i16) -> crate::error::Result<()> {
        // SAFETY: `val` references initialized and properly-aligned union storage.
        self.set_prop(FuelGaugeProp::SbsAtRate, |val| unsafe {
            *val.sbs_at_rate.as_mut() = value
        })
    }

    /// AtRateTimeToFull (minutes).
    pub fn sbs_at_rate_time_to_full(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::SbsAtRateTimeToFullMins)
            .map(|val| unsafe { *val.sbs_at_rate_time_to_full_mins.as_ref() })
    }

    /// AtRateTimeToEmpty (minutes).
    pub fn sbs_at_rate_time_to_empty(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::SbsAtRateTimeToEmptyMins)
            .map(|val| unsafe { *val.sbs_at_rate_time_to_empty_mins.as_ref() })
    }

    /// AtRateOK (boolean).
    pub fn sbs_at_rate_ok(&self) -> crate::error::Result<bool> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::SbsAtRateOk)
            .map(|val| unsafe { *val.sbs_at_rate_ok.as_ref() })
    }

    /// Remaining Capacity Alarm (mAh or 10mWh).
    pub fn sbs_remaining_capacity_alarm(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::SbsRemainingCapacityAlarm)
            .map(|val| unsafe { *val.sbs_remaining_capacity_alarm.as_ref() })
    }

    /// Sets the remaining Capacity Alarm (mAh or 10mWh).
    pub fn set_sbs_remaining_capacity_alarm(&self, value: u16) -> crate::error::Result<()> {
        // SAFETY: `val` references initialized and properly-aligned union storage.
        self.set_prop(FuelGaugeProp::SbsRemainingCapacityAlarm, |val| unsafe {
            *val.sbs_remaining_capacity_alarm.as_mut() = value
        })
    }

    /// Remaining Time Alarm (minutes).
    pub fn sbs_remaining_time_alarm(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::SbsRemainingTimeAlarmMins)
            .map(|val| unsafe { *val.sbs_remaining_time_alarm_mins.as_ref() })
    }

    /// Sets remaining Time Alarm (minutes).
    pub fn set_sbs_remaining_time_alarm(&self, value: u16) -> crate::error::Result<()> {
        // SAFETY: `val` references initialized and properly-aligned union storage.
        self.set_prop(FuelGaugeProp::SbsRemainingTimeAlarmMins, |val| unsafe {
            *val.sbs_remaining_time_alarm_mins.as_mut() = value
        })
    }

    /// Battery current direction (flags).
    pub fn current_direction(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::CurrentDirection)
            .map(|val| unsafe { *val.current_direction.as_ref() })
    }

    /// Remaining state of charge alarm (percent, 0-100).
    pub fn state_of_charge_alarm(&self) -> crate::error::Result<u8> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::StateOfChargeAlarmPct)
            .map(|val| unsafe { *val.state_of_charge_alarm_pct.as_ref() })
    }

    /// Low Cell Voltage Alarm (uV).
    pub fn low_voltage_alarm(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::LowVoltageAlarmUv)
            .map(|val| unsafe { *val.low_voltage_alarm_uv.as_ref() })
    }

    /// High Cell Voltage Alarm (uV).
    pub fn high_voltage_alarm(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::HighVoltageAlarmUv)
            .map(|val| unsafe { *val.high_voltage_alarm_uv.as_ref() })
    }

    /// Low Cell Current Alarm (uA).
    pub fn low_current_alarm(&self) -> crate::error::Result<i32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::LowCurrentAlarmUa)
            .map(|val| unsafe { *val.low_current_alarm_ua.as_ref() })
    }

    /// High Cell Current Alarm (uA).
    pub fn high_current_alarm(&self) -> crate::error::Result<i32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::HighCurrentAlarmUa)
            .map(|val| unsafe { *val.high_current_alarm_ua.as_ref() })
    }

    /// Low Cell Temperature Alarm (dK).
    pub fn low_temperature_alarm(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::LowTemperatureAlarmDk)
            .map(|val| unsafe { *val.low_temperature_alarm_dk.as_ref() })
    }

    /// High Cell Temperature Alarm (dK).
    pub fn high_temperature_alarm(&self) -> crate::error::Result<u16> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::HighTemperatureAlarmDk)
            .map(|val| unsafe { *val.high_temperature_alarm_dk.as_ref() })
    }

    /// GPIO Voltage (uV).
    pub fn gpio_voltage(&self) -> crate::error::Result<i32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::GpioVoltageUv)
            .map(|val| unsafe { *val.gpio_voltage_uv.as_ref() })
    }

    /// Low GPIO Voltage Alarm (uV).
    pub fn low_gpio_alarm(&self) -> crate::error::Result<i32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::LowGpioAlarmUv)
            .map(|val| unsafe { *val.low_gpio_alarm_uv.as_ref() })
    }

    /// High GPIO Voltage Alarm (uV).
    pub fn high_gpio_alarm(&self) -> crate::error::Result<i32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::HighGpioAlarmUv)
            .map(|val| unsafe { *val.high_gpio_alarm_uv.as_ref() })
    }

    /// ADC Mode (flags).
    pub fn adc_mode(&self) -> crate::error::Result<u8> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::AdcMode)
            .map(|val| unsafe { *val.adc_mode.as_ref() })
    }

    /// Coulomb Counter Config (flags).
    pub fn cc_config(&self) -> crate::error::Result<u8> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::CcConfig)
            .map(|val| unsafe { *val.cc_config.as_ref() })
    }

    /// State of Health (SoH) (percent, 0-100).
    pub fn state_of_health(&self) -> crate::error::Result<u8> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::StateOfHealth)
            .map(|val| unsafe { *val.state_of_health.as_ref() })
    }

    /// Thermistor Voltage Sense reading (uV).
    pub fn thermistor_voltage(&self) -> crate::error::Result<u32> {
        // SAFETY: Per the Zephyr API contract, the driver will have populated the correct field of the union.
        self.get_prop(FuelGaugeProp::ThermVoltageUv)
            .map(|val| unsafe { *val.therm_voltage_uv.as_ref() })
    }
}
